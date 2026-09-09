"use strict";

const REFRESH_INTERVAL_MS = 20_000;
let requestInFlight = false;

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
  current.replaceWith(next);
}

function ownerIsEditing() {
  const active = document.activeElement;
  const editing =
    active instanceof HTMLInputElement ||
    active instanceof HTMLSelectElement ||
    active instanceof HTMLTextAreaElement;
  return editing || document.querySelector("form[data-submitting='true']") !== null;
}

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

document.addEventListener("submit", (event) => {
  const form = event.target instanceof HTMLFormElement ? event.target : null;
  if (!form?.matches("form[data-live-form]")) return;
  event.preventDefault();
  void submitLive(form);
});

document.addEventListener("visibilitychange", () => {
  if (!document.hidden) void refreshDashboard();
});

window.setInterval(() => void refreshDashboard({ quiet: true }), REFRESH_INTERVAL_MS);
