//! CLI Wi-Fi sync peer — the protocol test stand-in for the future Android
//! companion app (docs/epim_clone_plan.md §D1): pairs with a running Smart
//! PIM desktop (or discovers one over mDNS) and runs a full pull+push sync
//! session against a local Smart PIM database file.
//!
//! Usage:
//! ```text
//! sync_peer discover                                  # find servers on the LAN
//! sync_peer pair   <db> <passphrase> <host> <port> <code> <name>
//! sync_peer sync   <db> <passphrase> <host> <port> <server-device-id>
//! ```

use smart_pim_lib::wifisync;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("discover") => discover(),
        Some("pair") => pair(
            &args[1],
            &args[2],
            &args[3],
            args[4].parse().expect("port"),
            &args[5],
            &args[6],
        ),
        Some("sync") => sync(
            &args[1],
            &args[2],
            &args[3],
            args[4].parse().expect("port"),
            &args[5],
        ),
        _ => {
            eprintln!("usage: sync_peer discover | pair <db> <pass> <host> <port> <code> <name> | sync <db> <pass> <host> <port> <server-id>");
            std::process::exit(2);
        }
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn discover() -> Result<(), String> {
    let peers = wifisync::discover(3000)?;
    if peers.is_empty() {
        println!("no Smart PIM sync servers found on the LAN");
        return Ok(());
    }
    for p in peers {
        println!(
            "{}\t{}\t{}:{}, name={}",
            p.device_id, p.platform, p.host, p.port, p.name
        );
    }
    Ok(())
}

fn pair(db: &str, pass: &str, host: &str, port: u16, code: &str, name: &str) -> Result<(), String> {
    let store = pfcore::Store::open(std::path::Path::new(db), pass).map_err(|e| e.to_string())?;
    let id = wifisync::self_id(store.conn());
    let outcome = wifisync::pair(host, port, code, &id, name)?;
    {
        let conn = store.conn();
        wifisync::setting_set(
            conn,
            &wifisync::token_key(&outcome.server_id),
            &outcome.token,
        );
        wifisync::setting_set(conn, &format!("sync.host.{}", outcome.server_id), host);
        wifisync::setting_set(
            conn,
            &format!("sync.port.{}", outcome.server_id),
            &port.to_string(),
        );
        pfcore::sync::device_register(conn, &outcome.server_id, name, "cli")
            .map_err(|e| e.to_string())?;
    }
    println!("paired with server {} (token stored)", outcome.server_id);
    Ok(())
}

fn sync(db: &str, pass: &str, host: &str, port: u16, server_id: &str) -> Result<(), String> {
    let mut store =
        pfcore::Store::open(std::path::Path::new(db), pass).map_err(|e| e.to_string())?;
    let summary = wifisync::sync_with(&mut store, host, port, server_id)?;
    println!(
        "sync complete: pulled={} pushed={} conflicts={} skipped={}",
        summary.pulled, summary.pushed, summary.conflicts, summary.skipped
    );
    Ok(())
}
