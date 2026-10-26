//! Manage → Import: bank CSV preview and confirmed import (personal-site
//! `owner-operations.tsx`, "CSV import").

use finance_types::manage::{CsvCommit, CsvCommitResult, CsvPreview, CsvUpload, ManageWorkspace};
use finance_types::report::Currency;
use leptos::html::Input;
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;
use wasm_bindgen::prelude::*;

use super::ui::{CONTROLS, FIELDSET, FILE_INPUT, Ops, frame, load_workspace};
use crate::ds::{
    Button, CONTROL, Card, CardDescription, CardTitle, CardTone, Checkbox, Eyebrow, Field, Heading,
    Money, Select, TABLE, TABLE_CONTAINER, TCELL, TCELL_NUM, TH, TH_NUM, TROW, Variant, cn,
};
use crate::state;

#[wasm_bindgen(inline_js = "
export async function read_bank_export(input) {
  const file = input.files && input.files[0];
  if (!file) throw 'Choose a CSV file.';
  if (!file.size || file.size > 5 * 1024 * 1024 || file.name.length > 255)
    throw 'Choose an account and a CSV file up to 5 MB.';
  const bytes = new Uint8Array(await file.arrayBuffer());
  let text;
  try {
    text = new TextDecoder('utf-8', { fatal: true }).decode(bytes);
  } catch {
    // Python fell back to cp1252, which has no characters for these bytes.
    if ([0x81, 0x8d, 0x8f, 0x90, 0x9d].some((b) => bytes.includes(b)))
      throw 'CSV encoding is not supported.';
    text = new TextDecoder('windows-1252').decode(bytes);
  }
  return [file.name, text];
}
")]
extern "C" {
    /// The chosen file's name and text, decoded as Python did (UTF-8 with an
    /// optional BOM, else Windows-1252).
    #[wasm_bindgen(catch)]
    async fn read_bank_export(input: &web_sys::HtmlInputElement) -> Result<JsValue, JsValue>;
}

#[derive(Serialize)]
struct UploadArgs {
    upload: CsvUpload,
}

#[derive(Serialize)]
struct CommitArgs {
    commit: CsvCommit,
}

#[component]
pub fn Import() -> impl IntoView {
    let ops = Ops::new();
    let retry = RwSignal::new(0_u32);
    let workspace = load_workspace(retry);
    let account = RwSignal::new("auto".to_owned());
    let has_file = RwSignal::new(false);
    let csv = RwSignal::new(None::<CsvPreview>);
    let skip = RwSignal::new(false);
    let file_input = NodeRef::<Input>::new();
    // A new account choice invalidates the preview, as in the site.
    Effect::new(move |previous: Option<String>| {
        let current = account.get();
        if previous.is_some_and(|p| p != current) {
            csv.set(None);
        }
        current
    });
    frame(ops, workspace, retry, move |ws| {
        view! { <ImportCard ws=ws ops=ops account=account has_file=has_file csv=csv skip=skip file_input=file_input /> }
            .into_any()
    })
}

#[component]
fn ImportCard(
    ws: ManageWorkspace,
    ops: Ops,
    account: RwSignal<String>,
    has_file: RwSignal<bool>,
    csv: RwSignal<Option<CsvPreview>>,
    skip: RwSignal<bool>,
    file_input: NodeRef<Input>,
) -> impl IntoView {
    let app = state();
    let busy = ops.busy;
    let mut options = vec![("auto".to_owned(), "Detect automatically".to_owned())];
    options.extend(
        ws.accounts
            .iter()
            .map(|a| (a.id.clone(), format!("{} · {}", a.name, a.currency))),
    );
    let submit = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        let Some(input) = file_input.get() else {
            return;
        };
        if !has_file.get_untracked() || !ops.begin() {
            return;
        }
        csv.set(None);
        skip.set(false);
        let account_id = account.get_untracked();
        spawn_local(async move {
            let read = read_bank_export(&input).await;
            let upload = read
                .map_err(|e| {
                    e.as_string()
                        .unwrap_or_else(|| "The file could not be read.".into())
                })
                .map(|pair| {
                    let pair = js_sys::Array::from(&pair);
                    CsvUpload {
                        account_id,
                        filename: pair.get(0).as_string().unwrap_or_default(),
                        content: pair.get(1).as_string().unwrap_or_default(),
                    }
                });
            match upload {
                Ok(upload) => {
                    match crate::api::invoke::<CsvPreview>(
                        "preview_csv_import",
                        &UploadArgs { upload },
                    )
                    .await
                    {
                        Ok(preview) => csv.set(Some(preview)),
                        Err(e) => ops.error.set(e),
                    }
                }
                Err(e) => ops.error.set(e),
            }
            ops.busy.set(false);
        });
    };
    let confirm = move |_| {
        let Some(preview) = csv.get_untracked() else {
            return;
        };
        let commit = CsvCommit {
            preview_id: preview.preview_id.clone(),
            skip_flagged: !preview.committable && skip.get_untracked(),
        };
        ops.run(
            "commit_csv_import",
            CommitArgs { commit },
            move |result: CsvCommitResult| {
                csv.set(None);
                has_file.set(false);
                if let Some(input) = file_input.get_untracked() {
                    input.set_value("");
                }
                ops.notice.set(format!(
                    "{} transactions imported.{}",
                    result.imported,
                    if result.refreshed {
                        " Reports refreshed."
                    } else {
                        " Import completed; report refresh is pending. Do not reimport."
                    }
                ));
                app.refresh();
            },
        );
    };
    view! {
        <Card>
            <Eyebrow level=Heading::P>"BANK CSV IMPORT"</Eyebrow>
            <CardTitle level=Heading::H2 class="mt-1">"Import transactions"</CardTitle>
            <CardDescription>"Choose a file, review the result, then confirm."</CardDescription>
            {if ws.csv_enabled {
                view! {
                    <form class="mt-5" on:submit=submit>
                        <fieldset disabled=move || busy.get() class=cn([FIELDSET, CONTROLS])>
                            <Field label="Destination account" id="csv-account" class="sm:w-64">
                                <Select id="csv-account" value=account options=Signal::derive(move || options.clone()) />
                            </Field>
                            <Field label="Bank export" id="csv-file" class="min-w-0 sm:flex-1">
                                <input
                                    id="csv-file"
                                    node_ref=file_input
                                    required
                                    type="file"
                                    class=cn([CONTROL, FILE_INPUT])
                                    accept=".csv,.txt,text/csv,text/plain"
                                    on:change=move |e| {
                                        let input: web_sys::HtmlInputElement = event_target(&e);
                                        has_file.set(input.files().is_some_and(|f| f.length() > 0));
                                        csv.set(None);
                                    }
                                />
                            </Field>
                            <Button submit=true disabled=Signal::derive(move || !has_file.get() || busy.get())>
                                {move || if busy.get() { "Working…" } else { "Preview" }}
                            </Button>
                        </fieldset>
                    </form>
                    <p class="mt-3 text-xs text-fg-muted">"CSV or text · maximum 5 MB · processed on this Mac"</p>
                    {move || csv.get().map(|p| view! { <PreviewView p=p ops=ops skip=skip csv=csv confirm=Callback::new(confirm) /> })}
                }
                    .into_any()
            } else {
                view! { <p class=cn([super::ui::FEEDBACK, "mt-5"])>"Import is unavailable."</p> }.into_any()
            }}
        </Card>
    }
}

#[component]
fn PreviewView(
    p: CsvPreview,
    ops: Ops,
    skip: RwSignal<bool>,
    csv: RwSignal<Option<CsvPreview>>,
    confirm: Callback<leptos::ev::MouseEvent>,
) -> impl IntoView {
    let busy = ops.busy;
    let counts = [
        (p.new_count, "new"),
        (p.duplicate_count, "exact duplicates"),
        (p.possible_duplicate_count, "possible duplicates"),
        (p.rejected_count, "unreadable"),
    ];
    let flagged = p.possible_duplicate_count + p.rejected_count;
    let more = (p.row_count as usize > p.rows.len())
        .then(|| format!("Showing {} of {} rows.", p.rows.len(), p.row_count));
    let committable = p.committable;
    let new_count = p.new_count;
    let rows = p.rows.clone();
    let rejected = p.rejected_rows.clone();
    view! {
        <div class="mt-8 grid gap-4">
            <h3 class="text-base font-semibold">{format!("{} → {}", p.bank_label, p.account_name)}</h3>
            <div class="grid grid-cols-2 gap-3 md:grid-cols-4">
                {counts
                    .into_iter()
                    .map(|(count, label)| {
                        view! {
                            <span class="rounded-lg border border-border bg-card-muted p-4 text-sm">
                                <b class="mb-1 block text-2xl font-semibold tabular-nums">{count}</b>
                                " "
                                {label}
                            </span>
                        }
                    })
                    .collect_view()}
            </div>
            <div class=TABLE_CONTAINER tabindex="0" role="region" aria-label="CSV import preview">
                <table class=TABLE data-density="md">
                    <thead class="text-left">
                        <tr>
                            <th class=TH>"Line"</th>
                            <th class=TH>"Date / description"</th>
                            <th class=TH_NUM>"Amount"</th>
                            <th class=TH>"Status"</th>
                        </tr>
                    </thead>
                    <tbody>
                        {rows
                            .into_iter()
                            .map(|r| {
                                view! {
                                    <tr class=TROW>
                                        <td class=cn([TCELL, "text-fg-muted tabular-nums"])>{r.line_number}</td>
                                        <td class=TCELL>
                                            <strong class="block font-semibold">{r.description}</strong>
                                            <small class="mt-1 block text-xs text-fg-muted">{r.booked_on}</small>
                                        </td>
                                        <td class=cn([TCELL_NUM, "whitespace-nowrap"])>
                                            <Money value=r.amount unit=Currency::new(&r.currency) neutral=true />
                                        </td>
                                        <td class=TCELL>{r.status.label()}</td>
                                    </tr>
                                }
                            })
                            .collect_view()}
                    </tbody>
                </table>
            </div>
            {more.map(|m| view! { <p class="text-sm text-fg-muted">{m}</p> })}
            {rejected
                .into_iter()
                .map(|r| {
                    view! {
                        <p class="text-sm text-warning-fg">
                            {format!("Line {}: {}", r.line_number, r.reason.replace('_', " "))}
                        </p>
                    }
                })
                .collect_view()}
            {(!committable)
                .then(|| {
                    view! {
                        <Card tone=CardTone::Muted class="p-4 sm:p-4">
                            <label class="flex items-start gap-2.5 text-sm leading-relaxed">
                                <span class="mt-1 flex">
                                    <Checkbox
                                        checked=Signal::derive(move || skip.get())
                                        disabled=Signal::derive(move || busy.get())
                                        on_change=Callback::new(move |v| skip.set(v))
                                    />
                                </span>
                                <span>
                                    {format!(
                                        "I reviewed the flagged rows. Skip the {flagged} flagged rows and import only rows marked new.",
                                    )}
                                </span>
                            </label>
                        </Card>
                    }
                })}
            <div class="flex flex-wrap items-center gap-2">
                <Button
                    disabled=Signal::derive(move || busy.get() || new_count == 0 || (!committable && !skip.get()))
                    on_click=confirm
                >
                    {move || {
                        if busy.get() {
                            "Importing…".to_owned()
                        } else {
                            format!("Confirm import of {new_count} new transactions")
                        }
                    }}
                </Button>
                <Button
                    variant=Variant::Ghost
                    disabled=Signal::derive(move || busy.get())
                    on_click=Callback::new(move |_| {
                        csv.set(None);
                        skip.set(false);
                    })
                >
                    "Discard preview"
                </Button>
            </div>
        </div>
    }
}
