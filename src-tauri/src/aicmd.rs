//! AI command surface (docs/ai_integ_plan.md §4): status, settings, job
//! queue, semantic search, plus the pure-computation helpers (free slots,
//! contact dedupe) and tag add/remove. Errors cross the bridge as strings.

use serde_json::{json, Value};
use tauri::State;

use crate::state::AppState;

fn with_store<T>(
    state: &State<AppState>,
    f: impl FnOnce(&mut pfcore::Store) -> Result<T, String>,
) -> Result<T, String> {
    let mut guard = state.store.lock().expect("store mutex");
    let Some(store) = guard.as_mut() else {
        return Err("database is locked".to_string());
    };
    f(store)
}

/// Provider reachability + effective settings + feature flags + embedding
/// progress. Never includes the API key.
#[tauri::command(async)]
pub fn ai_status(state: State<AppState>) -> Result<Value, String> {
    let (cfg, features, models, embedded, embeddable) = with_store(&state, |st| {
        let conn = st.conn();
        let cfg = crate::ai::load_config(conn);
        let features = crate::ai::features(conn);
        let models = crate::ai::list_models(
            &crate::ai::UreqHttp,
            &cfg,
            crate::ai::api_key(conn).as_deref(),
        );
        let embeddable: i64 = conn
            .query_row(
                "SELECT count(*) FROM item WHERE deleted_at IS NULL \
                 AND kind IN ('note','task','diary')",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let embedded: i64 = conn
            .query_row(
                "SELECT count(*) FROM embedding WHERE model = ?1",
                [&cfg.model],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        Ok((cfg, features, models, embedded, embeddable))
    })?;
    let key_set = with_store(&state, |st| Ok(crate::ai::api_key(st.conn()).is_some()))?;
    Ok(json!({
        "provider": cfg.provider,
        "endpoint": cfg.endpoint,
        "model": cfg.model,
        "cloud_enabled": cfg.cloud_enabled,
        "api_key_set": key_set,
        "reachable": !models.is_empty(),
        "models": models,
        "error": if models.is_empty() { Some("no model server found at the endpoint".to_string()) } else { None },
        "features": features,
        "embedded": embedded,
        "embeddable": embeddable,
    }))
}

#[tauri::command(async)]
pub fn ai_settings_get(state: State<AppState>) -> Result<Value, String> {
    with_store(&state, |st| {
        let conn = st.conn();
        let cfg = crate::ai::load_config(conn);
        Ok(json!({
            "provider": cfg.provider,
            "endpoint": cfg.endpoint,
            "model": cfg.model,
            "cloud_enabled": cfg.cloud_enabled,
            "api_key_set": crate::ai::api_key(conn).is_some(),
            "features": crate::ai::features(conn),
            "cost_per_1k_in": crate::ai::cost_rate(conn, crate::ai::COST_IN_KEY),
            "cost_per_1k_out": crate::ai::cost_rate(conn, crate::ai::COST_OUT_KEY),
        }))
    })
}

/// Apply a settings patch. `api_key: null` clears the stored key.
#[tauri::command(async)]
pub fn ai_settings_set(state: State<AppState>, patch: Value) -> Result<(), String> {
    with_store(&state, |st| {
        let conn = st.conn();
        let set = |key: &str, value: String| {
            conn.execute(
                "INSERT INTO setting (key, value) VALUES (?1, ?2) \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                rusqlite::params![key, value],
            )
            .map_err(|e| e.to_string())
        };
        if let Some(p) = patch["provider"].as_str() {
            if !matches!(p, "local" | "cloud") {
                return Err(format!("unknown provider {p}"));
            }
            set(crate::ai::PROVIDER_KEY, p.to_string())?;
        }
        if let Some(e) = patch["endpoint"].as_str() {
            set(crate::ai::ENDPOINT_KEY, e.to_string())?;
        }
        if let Some(m) = patch["model"].as_str() {
            set(crate::ai::MODEL_KEY, m.to_string())?;
        }
        if let Some(c) = patch["cloud_enabled"].as_bool() {
            set(
                crate::ai::CLOUD_KEY,
                if c { "1".into() } else { "0".into() },
            )?;
        }
        if let Some(k) = patch.get("api_key") {
            if k.is_null() {
                conn.execute(
                    "DELETE FROM setting WHERE key = ?1",
                    [crate::ai::API_KEY_REF],
                )
                .map_err(|e| e.to_string())?;
            } else {
                let key = k.as_str().ok_or("api_key must be a string or null")?;
                set(crate::ai::API_KEY_REF, key.to_string())?;
            }
        }
        if let Some(features) = patch["features"].as_object() {
            for (name, v) in features {
                let flag = v.as_bool().ok_or("feature flags must be booleans")?;
                set(
                    &format!("ai.feature.{name}"),
                    if flag { "1".into() } else { "0".into() },
                )?;
            }
        }
        for (field, key) in [
            ("cost_per_1k_in", crate::ai::COST_IN_KEY),
            ("cost_per_1k_out", crate::ai::COST_OUT_KEY),
        ] {
            if let Some(v) = patch.get(field) {
                if v.is_null() {
                    conn.execute("DELETE FROM setting WHERE key = ?1", [key])
                        .map_err(|e| e.to_string())?;
                } else {
                    let s = v.as_f64().ok_or("cost rates must be numbers or null")?;
                    if s < 0.0 {
                        return Err("cost rates cannot be negative".into());
                    }
                    set(key, format!("{s}"))?;
                }
            }
        }
        Ok(())
    })
}

/// Enqueue an AI job. Feature-gated; returns the job id — completion arrives
/// via the `ai://job` event (poll `ai_job` as fallback).
#[tauri::command(async)]
pub fn ai_run(state: State<AppState>, kind: String, params: Value) -> Result<i64, String> {
    with_store(&state, |st| {
        if !crate::ai::feature_allows(st.conn(), &kind) {
            return Err(format!("AI feature is turned off: {kind} (Settings → AI)"));
        }
        crate::aiworker::enqueue(st.conn(), &kind, &params).map_err(|e| e.to_string())
    })
}

/// A job row (poll fallback for the `ai://job` event).
#[tauri::command(async)]
pub fn ai_job(state: State<AppState>, job_id: i64) -> Result<Value, String> {
    with_store(&state, |st| {
        crate::aiworker::job_row(st.conn(), job_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("no job {job_id}"))
    })
}

#[tauri::command(async)]
pub fn ai_cancel(state: State<AppState>, job_id: i64) -> Result<(), String> {
    with_store(&state, |st| {
        crate::aiworker::cancel(st.conn(), job_id, &state.ai_cancel).map_err(|e| e.to_string())
    })
}

/// Enqueue a full embedding rebuild (semantic feature).
#[tauri::command(async)]
pub fn ai_reindex(state: State<AppState>) -> Result<i64, String> {
    with_store(&state, |st| {
        if !crate::ai::feature_allows(st.conn(), "reindex") {
            return Err("AI feature is turned off: semantic (Settings → AI)".into());
        }
        crate::aiworker::enqueue(st.conn(), "reindex", &json!({})).map_err(|e| e.to_string())
    })
}

/// Semantic search: embed the query, rank candidates by cosine, resolve to
/// search hits. FTS stays the default path — this is the explicit toggle.
#[tauri::command(async)]
pub fn ai_search_semantic(
    state: State<AppState>,
    query: String,
    limit: i64,
) -> Result<Value, String> {
    if query.trim().is_empty() {
        return Err("empty query".into());
    }
    let hits = with_store(&state, |st| {
        let conn = st.conn();
        let cfg = crate::ai::load_config(conn);
        if cfg.model.is_empty() {
            return Err("no AI model configured (Settings → AI)".into());
        }
        let key = crate::ai::api_key(conn);
        let vectors = crate::ai::embed(
            &crate::ai::UreqHttp,
            &cfg,
            key.as_deref(),
            std::slice::from_ref(&query),
        )
        .map_err(|e| e.to_string())?;
        let Some(qv) = vectors.into_iter().next() else {
            return Err("empty embedding response".into());
        };
        let candidates =
            pfcore::aisearch::semantic_candidates(conn, &cfg.model, &qv, limit.max(1) as usize)
                .map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for (item_id, score) in candidates {
            let Some((kind, title, snippet)) = resolve_hit(conn, item_id) else {
                continue;
            };
            out.push(json!({
                "kind": kind, "id": item_id, "title": title,
                "snippet": snippet, "score": (score * 1000.0).round() / 1000.0,
            }));
        }
        Ok(Value::Array(out))
    })?;
    Ok(hits)
}

/// Resolve an item id to a search hit triple (kind, title, snippet).
fn resolve_hit(
    conn: &rusqlite::Connection,
    item_id: i64,
) -> Option<(String, String, Option<String>)> {
    let kind: String = conn
        .query_row(
            "SELECT kind FROM item WHERE id = ?1 AND deleted_at IS NULL",
            [item_id],
            |r| r.get(0),
        )
        .ok()?;
    match kind.as_str() {
        "note" => conn
            .query_row(
                "SELECT title, substr(content, 1, 160) FROM note WHERE item_id = ?1",
                [item_id],
                |r| Ok((r.get::<_, String>(0)?, Some(r.get::<_, String>(1)?))),
            )
            .ok()
            .map(|(title, snippet)| ("note".into(), title, snippet)),
        "task" => conn
            .query_row(
                "SELECT title, COALESCE(substr(notes, 1, 160), '') FROM task WHERE item_id = ?1",
                [item_id],
                |r| Ok((r.get::<_, String>(0)?, Some(r.get::<_, String>(1)?))),
            )
            .ok()
            .map(|(title, snippet)| ("task".into(), title, snippet)),
        "diary" => conn
            .query_row(
                "SELECT entry_date, substr(content, 1, 160) FROM diary_entry WHERE item_id = ?1",
                [item_id],
                |r| Ok((r.get::<_, String>(0)?, Some(r.get::<_, String>(1)?))),
            )
            .ok()
            .map(|(date, snippet)| ("diary".into(), date, snippet)),
        _ => None,
    }
}

/// Find-a-slot over the calendar (pure computation; plan §3).
#[tauri::command(async)]
pub fn free_slots(
    state: State<AppState>,
    calendar_id: Option<i64>,
    from: String,
    to: String,
    duration_min: i64,
) -> Result<Value, String> {
    with_store(&state, |st| {
        let slots = pfcore::schedule::free_slots(
            st.conn(),
            calendar_id,
            &from,
            &to,
            duration_min,
            None,
            20,
        )
        .map_err(|e| e.to_string())?;
        serde_json::to_value(slots).map_err(|e| e.to_string())
    })
}

/// Deterministic duplicate-contact candidates.
#[tauri::command(async)]
pub fn dupe_candidates(state: State<AppState>) -> Result<Value, String> {
    with_store(&state, |st| {
        let groups = pfcore::contacts::dupe_candidates(st.conn()).map_err(|e| e.to_string())?;
        serde_json::to_value(groups).map_err(|e| e.to_string())
    })
}

/// Merge `secondary_id` into `primary_id` (loser preserved in history).
#[tauri::command(async)]
pub fn merge_contacts(
    state: State<AppState>,
    primary_id: i64,
    secondary_id: i64,
) -> Result<i64, String> {
    with_store(&state, |st| {
        pfcore::contacts::merge_contact(st.conn(), primary_id, secondary_id)
            .map_err(|e| e.to_string())
    })
}

#[tauri::command(async)]
pub fn item_tag_add(state: State<AppState>, item_id: i64, tag: String) -> Result<(), String> {
    with_store(&state, |st| {
        pfcore::repo::tag_item(st.conn(), item_id, &tag).map_err(|e| e.to_string())
    })
}

#[tauri::command(async)]
pub fn item_tag_remove(state: State<AppState>, item_id: i64, tag: String) -> Result<(), String> {
    with_store(&state, |st| {
        pfcore::repo::untag_item(st.conn(), item_id, &tag).map_err(|e| e.to_string())
    })
}

/// Token/cost accounting (plan §4): per-feature-kind totals over `ai_job`
/// plus the effective per-1k cost rates, so Settings can show an estimated
/// spend. Never includes content — counters only.
#[tauri::command(async)]
pub fn ai_usage(state: State<AppState>) -> Result<Value, String> {
    with_store(&state, |st| {
        let conn = st.conn();
        let mut stmt = conn
            .prepare(
                "SELECT kind, count(*), sum(tokens_in), sum(tokens_out) \
                 FROM ai_job GROUP BY kind",
            )
            .map_err(|e| e.to_string())?;
        let mut by_kind = serde_json::Map::new();
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        let (mut jobs, mut tin, mut tout) = (0i64, 0i64, 0i64);
        for (kind, n, i, o) in rows.flatten() {
            jobs += n;
            tin += i;
            tout += o;
            by_kind.insert(kind, json!({"jobs": n, "tokens_in": i, "tokens_out": o}));
        }
        let rate = |key: &str| -> Option<f64> {
            conn.query_row("SELECT value FROM setting WHERE key = ?1", [key], |r| {
                r.get::<_, String>(0)
            })
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|f| *f >= 0.0)
        };
        let (rin, rout) = (rate(crate::ai::COST_IN_KEY), rate(crate::ai::COST_OUT_KEY));
        let est: Option<f64> = match (rin, rout) {
            (Some(ri), Some(ro)) => Some((tin as f64 / 1000.0) * ri + (tout as f64 / 1000.0) * ro),
            _ => None,
        };
        Ok(json!({
            "jobs": jobs,
            "tokens_in": tin,
            "tokens_out": tout,
            "by_kind": by_kind,
            "cost_per_1k_in": rin,
            "cost_per_1k_out": rout,
            "estimated_cost": est,
        }))
    })
}

/// Zero the usage counters (keeps job history).
#[tauri::command(async)]
pub fn ai_usage_reset(state: State<AppState>) -> Result<(), String> {
    with_store(&state, |st| {
        st.conn()
            .execute("UPDATE ai_job SET tokens_in = 0, tokens_out = 0", [])
            .map(|_| ())
            .map_err(|e| e.to_string())
    })
}
