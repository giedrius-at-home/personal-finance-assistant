// Synthetic DOM doubles: no browser, real accounts, or third-party dependencies.
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const { test } = require("node:test");

function load(document, extras = {}) {
  const handlers = {};
  const context = {
    document: { addEventListener: (type, fn) => (handlers[type] ||= []).push(fn), ...document },
    window: { setInterval() {} },
    HTMLInputElement: class {}, HTMLSelectElement: class {}, HTMLTextAreaElement: class {},
    URLSearchParams, CSS: { escape: value => value }, ...extras,
  };
  vm.createContext(context);
  vm.runInContext(fs.readFileSync(path.join(__dirname, "../../src/finance_assistant/static/dashboard.js"), "utf8"), context);
  return { context, handlers };
}

function form(category, value, revision) {
  const message = { textContent: "Suggestion only" };
  return {
    dataset: { groupCategory: category },
    elements: { group: { value, options: [{ value: "Groceries" }] }, revision: { value: revision },
      csrf_token: { value: "synthetic-token" }, category: { value: category } },
    querySelector: () => message,
  };
}

test("refresh preserves open groups, search and other unsaved forms with original revision", () => {
  const draft = form("Synthetic store", "Groceries", "2");
  const replacement = form("Synthetic store", "Other", "3");
  const nextSearch = { value: "" };
  const details = { open: false };
  let replaced = false;
  const next = {
    querySelector: selector => selector === "#group-search" ? nextSearch : details,
    querySelectorAll: () => [replacement],
  };
  const current = {
    querySelector: () => ({ value: "Synthetic" }),
    querySelectorAll: selector => selector.startsWith("details") ? [{ id: "group" }] : [draft],
    replaceWith: value => { assert.equal(value, next); replaced = true; },
  };
  const { context } = load({ querySelector: () => current, querySelectorAll: () => [replacement] }, {
    DOMParser: class { parseFromString() { return { querySelector: () => next }; } },
  });
  context.replaceDashboard("synthetic html");
  assert.ok(replaced && details.open);
  assert.equal(replacement.elements.group.value, "Groceries");
  assert.equal(replacement.elements.revision.value, "2");
  assert.equal(replacement.dataset.dirty, "true");
  assert.equal(nextSearch.value, "Synthetic");
  assert.equal(replacement.hidden, false);
  context.filterGroupMappings("absent");
  assert.equal(replacement.hidden, true);
});

test("local suggestion fills choice without saving; failed request leaves it intact", async () => {
  const draft = form("Synthetic store", "unassigned", "0");
  const button = { disabled: false, closest: () => draft };
  let fail = false;
  const requests = [];
  const { handlers } = load({}, { fetch: async (url, options) => {
    requests.push({ url, options });
    return { ok: !fail, json: async () => ({ group: "Groceries" }) };
  } });
  const event = { target: { closest: () => button } };
  await handlers.click[0](event);
  assert.equal(requests[0].url, "/spending-groups/suggest");
  assert.equal(draft.elements.group.value, "Groceries");
  assert.equal(draft.dataset.dirty, "true");
  assert.match(draft.querySelector().textContent, /not saved/);
  assert.equal(button.disabled, false);
  fail = true;
  await handlers.click[0](event);
  assert.equal(draft.elements.group.value, "Groceries");
  assert.match(draft.querySelector().textContent, /unavailable/);
  assert.ok(requests.every(request => request.url.endsWith("/suggest")));
});
