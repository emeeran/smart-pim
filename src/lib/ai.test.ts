import { describe, expect, it } from "vitest";
import {
  featureEnabled,
  parseActionItems,
  parseNlTask,
  parseReplyDraft,
  parseSignatureContact,
  parseSubtasks,
  parseSuggestedRules,
  priorityFrom,
  resultText,
} from "./ai";
import type { AiStatus } from "../api";

function status(patch: Partial<AiStatus>): AiStatus {
  return {
    provider: "local",
    endpoint: "http://127.0.0.1:11434",
    model: "llama3",
    cloud_enabled: false,
    api_key_set: false,
    features: { summarize: true },
    reachable: true,
    models: ["llama3"],
    error: null,
    embedded: 0,
    embeddable: 0,
    ...patch,
  };
}

describe("featureEnabled", () => {
  it("requires reachability AND the per-feature opt-in", () => {
    expect(featureEnabled(status({}), "summarize")).toBe(true);
    expect(featureEnabled(status({ features: {} }), "summarize")).toBe(false);
    expect(featureEnabled(status({ reachable: false }), "summarize")).toBe(false);
    expect(featureEnabled(null, "summarize")).toBe(false);
  });

  it("hides features whose key is missing entirely", () => {
    expect(featureEnabled(status({}), "diary")).toBe(false);
  });
});

describe("resultText", () => {
  it("unwraps common result envelopes", () => {
    expect(resultText("plain")).toBe("plain");
    expect(resultText({ summary: "s" })).toBe("s");
    expect(resultText({ digest: "d" })).toBe("d");
    expect(resultText({})).toBe("");
    expect(resultText(null)).toBe("");
  });
});

describe("parseActionItems", () => {
  it("reads an items array", () => {
    expect(parseActionItems({ items: [" call Bob ", "file taxes"] })).toEqual([
      "call Bob",
      "file taxes",
    ]);
  });

  it("accepts objects with a title field and drops empties", () => {
    expect(parseActionItems({ items: [{ title: "A" }, "", { nope: 1 }] })).toEqual(["A"]);
  });

  it("falls back to bullet/numbered text lines", () => {
    expect(parseActionItems("- buy milk\n2. post letter\n\n• call ana")).toEqual([
      "buy milk",
      "post letter",
      "call ana",
    ]);
  });
});

describe("parseNlTask", () => {
  it("maps title/priority/due onto task fields", () => {
    expect(
      parseNlTask({ title: "Call the dentist", priority: "urgent", due_utc: "2026-10-25T09:00:00Z" }),
    ).toEqual({
      title: "Call the dentist",
      priority: 3,
      due_utc: "2026-10-25T09:00:00.000Z",
    });
  });

  it("returns null without a title and tolerates garbage due dates", () => {
    expect(parseNlTask({ title: "  " })).toBeNull();
    expect(parseNlTask("nope")).toBeNull();
    expect(parseNlTask({ title: "X", due_utc: "nonsense" })?.due_utc).toBeNull();
  });
});

describe("priorityFrom", () => {
  it("maps names and clamps numbers", () => {
    expect(priorityFrom("low")).toBe(0);
    expect(priorityFrom("Urgent")).toBe(3);
    expect(priorityFrom(2)).toBe(2);
    expect(priorityFrom(99)).toBe(3);
    expect(priorityFrom("weird")).toBe(1);
    expect(priorityFrom(undefined)).toBe(1);
  });
});

describe("parseReplyDraft", () => {
  it("reads subject/body and rejects empty drafts", () => {
    expect(parseReplyDraft({ subject: "Re: hi", body: "Hello" })).toEqual({
      subject: "Re: hi",
      body: "Hello",
    });
    expect(parseReplyDraft({})).toBeNull();
    expect(parseReplyDraft("text only")).toEqual({ subject: "", body: "text only" });
    expect(parseReplyDraft("   ")).toBeNull();
  });
});

describe("parseSuggestedRules", () => {
  it("coerces rows and drops unusable ones", () => {
    const rules = parseSuggestedRules({
      rules: [
        { field: "from", op: "contains", value: "newsletter@", action: "move", action_value: "Newsletters" },
        { field: "bogus", op: "bogus", value: "", action: "bogus" },
        { op: "equals", value: "weekly", action: "mark_read", action_value: "x" },
      ],
    });
    expect(rules).toEqual([
      {
        account_id: null,
        field: "from",
        op: "contains",
        value: "newsletter@",
        action: "move",
        action_value: "Newsletters",
      },
      {
        account_id: null,
        field: "subject",
        op: "equals",
        value: "weekly",
        action: "mark_read",
        action_value: null,
      },
    ]);
  });

  it("returns [] for unexpected shapes", () => {
    expect(parseSuggestedRules(null)).toEqual([]);
    expect(parseSuggestedRules({ nope: 1 })).toEqual([]);
  });
});

describe("parseSubtasks", () => {
  it("reads the subtasks list, tolerating {title} objects", () => {
    expect(
      parseSubtasks({ subtasks: ["Draft outline", { title: " Review figures " }, ""] }),
    ).toEqual(["Draft outline", "Review figures"]);
  });

  it("falls back to a plain bullet list, like action items", () => {
    expect(parseSubtasks("one\ntwo")).toEqual(["one", "two"]);
    expect(parseSubtasks(null)).toEqual([]);
  });
});

describe("parseSignatureContact", () => {
  it("keeps editable fields and filters bad candidates", () => {
    const c = parseSignatureContact({
      first_name: " Ana ",
      last_name: "",
      organization: "Acme",
      emails: ["ana@example.com", ""],
      phones: ["+1 555 010 2030"],
      urls: [],
      candidates: [
        { contact_id: 7, display_name: "Ana Vera", organization: "Acme" },
        { contact_id: 0, display_name: "junk" },
        "junk",
      ],
    });
    expect(c).not.toBeNull();
    expect(c!.first_name).toBe("Ana");
    expect(c!.emails).toEqual(["ana@example.com"]);
    expect(c!.candidates).toEqual([
      { contact_id: 7, display_name: "Ana Vera", organization: "Acme" },
    ]);
  });

  it("renders nothing-usable as an empty proposal, not null", () => {
    const c = parseSignatureContact({});
    expect(c).not.toBeNull();
    expect(c!.emails).toEqual([]);
  });
});
