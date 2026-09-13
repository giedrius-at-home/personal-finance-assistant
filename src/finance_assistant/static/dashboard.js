"use strict";

const REFRESH_INTERVAL_MS = 20_000;
let requestInFlight = false;
let lastRevision = null;

function status(message, state = "ready") {
  const node = document.querySelector("#live-status");
  if (!node) return;
  node.textContent = message;
  node.dataset.state = state;
}

function replaceDashboard(html) {
  const parsed = new DOMParser().parseFromString(html, "text/html");
  const next = parsed.querySelector("#dashboard-content");
  const current = document.querySelector("#dashboard-content");
  if (!next || !current) throw new Error("dashboard fragment unavailable");
  for (const details of current.querySelectorAll("details[id][open]")) {
    const replacement = next.querySelector(`#${CSS.escape(details.id)}`);
    if (replacement) replacement.open = true;
  }
  const search = current.querySelector("#group-search")?.value || "";
  const drafts = new Map([...current.querySelectorAll(".group-mapping[data-dirty='true']:not([data-submitting='true'])")]
    .map(form => [form.dataset.groupCategory, form]));
  for (const replacement of next.querySelectorAll(".group-mapping")) {
    const draft = drafts.get(replacement.dataset.groupCategory);
    if (!draft) continue;
    replacement.elements.group.value = draft.elements.group.value;
    replacement.elements.revision.value = draft.elements.revision.value;
    replacement.dataset.dirty = "true";
    replacement.querySelector(".group-suggestion-status").textContent =
      draft.querySelector(".group-suggestion-status").textContent || "Unsaved choice.";
  }
  current.replaceWith(next);
  const nextSearch = next.querySelector("#group-search");
  if (nextSearch) nextSearch.value = search;
  filterGroupMappings(search);
}

function filterGroupMappings(query) {
  for (const form of document.querySelectorAll("[data-group-category]")) {
    form.hidden = !form.dataset.groupCategory.toLocaleLowerCase().includes(query.toLocaleLowerCase());
  }
}

document.addEventListener("input", (event) => {
  if (event.target.id === "group-search") filterGroupMappings(event.target.value);
});

function ownerIsEditing() {
  const active = document.activeElement;
  const editing =
    active instanceof HTMLInputElement ||
    active instanceof HTMLSelectElement ||
    active instanceof HTMLTextAreaElement;
  return editing || document.querySelector("form[data-submitting='true'], form[data-dirty='true']") !== null;
}

document.addEventListener("change", (event) => {
  const form = event.target.closest("form.group-mapping");
  if (form) form.dataset.dirty = "true";
});

document.addEventListener("click", async (event) => {
  const button = event.target.closest("[data-suggest-group]");
  if (!button || requestInFlight) return;
  const form = button.closest("form");
  const message = form.querySelector(".group-suggestion-status");
  requestInFlight = true;
  button.disabled = true;
  message.textContent = "Asking the local model…";
  try {
    const body = new URLSearchParams();
    body.set("csrf_token", form.elements.csrf_token.value);
    body.set("category", form.elements.category.value);
    const response = await fetch("/spending-groups/suggest", {
      method: "POST", body, credentials: "same-origin", cache: "no-store",
    });
    if (!response.ok) throw new Error("suggestion unavailable");
    const result = await response.json();
    if (result.group !== null && [...form.elements.group.options].some(option => option.value === result.group)) {
      form.elements.group.value = result.group;
      form.dataset.dirty = "true";
      message.textContent = "Suggestion only—not saved. Review it, then select Save group.";
    } else {
      message.textContent = "No confident suggestion. Choose the group yourself.";
    }
  } catch (_error) {
    message.textContent = "Local model unavailable. You can still choose and save a group.";
  } finally {
    requestInFlight = false;
    button.disabled = false;
  }
});

async function refreshDashboard({ quiet = false } = {}) {
  if (requestInFlight || document.hidden || ownerIsEditing()) return;
  requestInFlight = true;
  if (!quiet) status("Refreshing…");
  try {
    const response = await fetch("/dashboard", {
      credentials: "same-origin",
      cache: "no-store",
      headers: { Accept: "text/html" },
    });
    if (!response.ok) throw new Error("dashboard refresh failed");
    replaceDashboard(await response.text());
    if (!quiet) status("Updated just now.");
  } catch (_error) {
    if (!quiet) status("Could not refresh. The current view is unchanged.", "error");
  } finally {
    requestInFlight = false;
  }
}

async function pollDashboardRevision() {
  if (requestInFlight || document.hidden || ownerIsEditing()) return;
  try {
    const response = await fetch("/v1/dashboard-revision", {
      credentials: "same-origin",
      cache: "no-store",
      headers: { Accept: "application/json" },
    });
    if (!response.ok) return;
    const revision = (await response.json()).revision;
    if (lastRevision === null) {
      lastRevision = revision;
    } else if (revision !== lastRevision) {
      await refreshDashboard({ quiet: true });
      lastRevision = revision;
    }
  } catch (_error) {
    // Background polling is best effort; explicit refresh still reports failures.
  }
}

async function submitLive(form) {
  if (requestInFlight) return;
  requestInFlight = true;
  form.dataset.submitting = "true";
  const successMessage = form.dataset.liveSuccess || "Saved successfully.";
  let responseStatus = 0;
  for (const button of form.querySelectorAll("button")) button.disabled = true;
  status("Saving…");
  try {
    const body = new URLSearchParams();
    for (const [key, value] of new FormData(form)) {
      if (typeof value === "string") body.append(key, value);
    }
    const response = await fetch(form.action, {
      method: "POST",
      body,
      credentials: "same-origin",
      cache: "no-store",
      headers: { Accept: "text/html" },
    });
    responseStatus = response.status;
    if (!response.ok) throw new Error("dashboard action failed");
    replaceDashboard(await response.text());
    const serverMessage = document.querySelector("#live-status")?.textContent?.trim();
    if (!serverMessage) status(successMessage);
  } catch (_error) {
    delete form.dataset.submitting;
    for (const button of form.querySelectorAll("button")) button.disabled = false;
    if (responseStatus === 409) {
      status("This item changed before it could be saved. Refresh and try again.", "error");
    } else if (responseStatus === 503) {
      status("Firefly did not confirm the category. Refresh before retrying.", "error");
    } else {
      status("Could not confirm the update. Refresh before retrying.", "error");
    }
  } finally {
    requestInFlight = false;
  }
}

function renderResearch(container, result) {
  container.replaceChildren();
  const document = result?.research?.documents?.[0];
  const message = document
    ? `${document.citation.title}: ${document.excerpt}`
    : "No reliable public match was found. Please categorize this one manually.";
  const text = window.document.createElement("p");
  text.textContent = message;
  container.append(text);
  if (document) {
    const citation = new URL(document.citation.url);
    if (citation.protocol === "https:" && citation.hostname === "en.wikipedia.org") {
      const link = window.document.createElement("a");
      link.href = citation.href;
      link.target = "_blank";
      link.rel = "noopener noreferrer";
      link.textContent = "Verify public source";
      container.append(link);
    }
  }
  const warning = window.document.createElement("small");
  warning.textContent = "Untrusted public result—verify it before choosing a category.";
  container.append(warning);
}

async function submitResearch(form) {
  if (requestInFlight) return;
  requestInFlight = true;
  form.dataset.submitting = "true";
  const container = form.querySelector(".research-result");
  const buttons = form.querySelectorAll("button");
  for (const button of buttons) button.disabled = true;
  status("Researching public merchant name…");
  try {
    const body = new URLSearchParams();
    for (const [key, value] of new FormData(form)) {
      if (typeof value === "string") body.append(key, value);
    }
    const response = await fetch(form.action, {
      method: "POST",
      body,
      credentials: "same-origin",
      cache: "no-store",
      headers: { Accept: "application/json" },
    });
    if (!response.ok) throw new Error("merchant research failed");
    renderResearch(container, await response.json());
    status("Public merchant lookup finished. Verify the result before categorizing.");
  } catch (_error) {
    status("Could not find a safe public merchant match. Categorize this one manually.", "error");
  } finally {
    delete form.dataset.submitting;
    for (const button of buttons) button.disabled = false;
    requestInFlight = false;
  }
}

document.addEventListener("submit", (event) => {
  const form = event.target instanceof HTMLFormElement ? event.target : null;
  if (form?.matches("form[data-research-form]")) {
    event.preventDefault();
    void submitResearch(form);
    return;
  }
  if (!form?.matches("form[data-live-form]")) return;
  event.preventDefault();
  void submitLive(form);
});

document.addEventListener("visibilitychange", () => {
  if (!document.hidden) void refreshDashboard();
});

window.setInterval(() => void pollDashboardRevision(), REFRESH_INTERVAL_MS);
