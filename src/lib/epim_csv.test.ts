import { describe, expect, it } from "vitest";
import { detectEpimCsv } from "./epim_csv";

describe("detectEpimCsv", () => {
  it("detects an EssentialPIM contacts export", () => {
    const m = detectEpimCsv([
      "First Name",
      "Last Name",
      "Display Name",
      "E-mail",
      "Home Phone",
      "Organization",
      "Home Address",
      "Web page",
      "Birthday",
      "Notes",
      "Categories",
    ]);
    expect(m?.preset).toBe("contacts");
    expect(m?.label).toBe("EssentialPIM Contacts export");
    expect(m?.mapping).toEqual({
      first_name: 0,
      last_name: 1,
      display_name: 2,
      email: 3,
      phone: 4,
      organization: 5,
      address: 6,
      url: 7,
      birthday: 8,
      notes: 9,
      categories: 10,
    });
  });

  it("detects an EssentialPIM tasks export", () => {
    const m = detectEpimCsv([
      "Task Subject",
      "Start Date",
      "Due Date",
      "Categories",
      "Percent Complete",
      "Priority",
      "Notes",
    ]);
    expect(m?.preset).toBe("tasks");
    expect(m?.mapping.subject).toBe(0);
    expect(m?.mapping.start_date).toBe(1);
    expect(m?.mapping.due_date).toBe(2);
    expect(m?.mapping.categories).toBe(3);
    expect(m?.mapping.percent_complete).toBe(4);
    expect(m?.mapping.priority).toBe(5);
  });

  it("detects an EssentialPIM notes export", () => {
    const m = detectEpimCsv(["Note Title", "Note Text", "Categories"]);
    expect(m?.preset).toBe("notes");
    expect(m?.mapping).toEqual({ title: 0, content: 1, categories: 2 });
  });

  it("detects an EssentialPIM appointments export with date+time split", () => {
    const m = detectEpimCsv([
      "Subject",
      "Start Date",
      "Start Time",
      "End Date",
      "End Time",
      "Location",
      "Description",
      "Categories",
      "Repeat",
    ]);
    expect(m?.preset).toBe("events");
    expect(m?.mapping).toEqual({
      subject: 0,
      start_date: 1,
      start_time: 2,
      end_date: 3,
      end_time: 4,
      location: 5,
      description: 6,
      categories: 7,
      repeat: 8,
    });
  });

  it("matches case-insensitively and through separators/whitespace", () => {
    const m = detectEpimCsv([" first_name ", "E-Mail 1", "Company", "Web Page"]);
    expect(m?.preset).toBe("contacts");
    expect(m?.mapping.first_name).toBe(0);
    expect(m?.mapping.email).toBe(1);
    expect(m?.mapping.organization).toBe(2);
    expect(m?.mapping.url).toBe(3);
  });

  it("does not reuse one column for two fields", () => {
    // "Notes" satisfies contacts.notes first; description must not steal it.
    const m = detectEpimCsv(["Subject", "Start Date", "Start Time", "Notes", "Location"]);
    expect(m?.preset).toBe("events");
    expect(m?.mapping.description).toBe(3);
    expect(m?.mapping.location).toBe(4);
    expect(Object.values(m?.mapping ?? {}).filter((v) => v === 3)).toHaveLength(1);
  });

  it("returns null for a generic CSV that matches no preset", () => {
    expect(detectEpimCsv(["a", "b", "c"])).toBeNull();
    expect(detectEpimCsv([])).toBeNull();
  });

  it("returns null when only the threshold is not reached", () => {
    // Two contacts-ish columns is below the contacts threshold (3).
    expect(detectEpimCsv(["First Name", "Last Name"])).toBeNull();
  });

  it("prefers the more distinctive layout on ties", () => {
    // "Subject, Start Date" reaches the tasks threshold but not events (3).
    expect(detectEpimCsv(["Subject", "Start Date"])?.preset).toBe("tasks");
  });
});
