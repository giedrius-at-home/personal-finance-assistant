//! "Research an unfamiliar merchant" (personal-site `MerchantResearch.tsx`)
//! and "Request local category suggestions" (`FinancePanel.tsx`).

use finance_types::ai::{MerchantResearchResult, SuggestionPass};
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::Serialize;

use crate::api::{NoArgs, invoke};
use crate::ds::patterns::{ACTION_BAR, ERROR_LINE, MUTED};
use crate::ds::{Button, Field, Help, Input, Variant};
use crate::state;

#[derive(Serialize)]
struct ResearchArgs {
    proposal: String,
    public_name: String,
}

#[component]
pub fn MerchantResearch(#[prop(into)] id: String) -> impl IntoView {
    let id = StoredValue::new(id);
    let name = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let error = RwSignal::new(String::new());
    let result = RwSignal::new(None::<MerchantResearchResult>);
    let submit = move |e: leptos::ev::SubmitEvent| {
        e.prevent_default();
        busy.set(true);
        error.set(String::new());
        result.set(None);
        let args = ResearchArgs {
            proposal: id.get_value(),
            public_name: name.get_untracked(),
        };
        spawn_local(async move {
            match invoke::<MerchantResearchResult>("research_merchant", &args).await {
                Ok(r) => result.set(Some(r)),
                Err(_) => {
                    error.set("Research is unavailable or the public name was rejected.".into())
                }
            }
            busy.set(false);
        });
    };
    let field_id = format!("research-{}", id.get_value().get(..12).unwrap_or(""));
    view! {
        <form class="grid gap-3 rounded-xl border border-border bg-card-strong p-5 text-fg" on:submit=submit>
            <Field
                label="Research an unfamiliar merchant"
                id=field_id.clone()
                hint="Only this public name is sent to the configured research provider. Leave out account and transaction details."
            >
                <Input id=field_id value=name placeholder="Public business name only" />
            </Field>
            <Button submit=true variant=Variant::Secondary class="justify-self-start" disabled=Signal::derive(move || busy.get() || name.with(|n| n.trim().chars().count() < 2))>
                {move || if busy.get() { "Researching…" } else { "Research public merchant" }}
            </Button>
            {move || {
                let e = error.get();
                (!e.is_empty()).then(|| view! { <p role="alert" class=ERROR_LINE>{e}</p> })
            }}
            {move || result.get().map(|r| {
                let status = r
                    .likely_business_type
                    .clone()
                    .unwrap_or_else(|| {
                        use finance_types::ai::ResearchStatus as S;
                        match r.research.status {
                            S::Live => "live",
                            S::FreshCache => "fresh cache",
                            S::StaleCache => "stale cache",
                            S::Disabled => "disabled",
                            S::Unavailable => "unavailable",
                            S::Abstained => "abstained",
                        }
                        .to_owned()
                    });
                view! {
                    <div class="grid gap-2 text-xs text-fg-muted">
                        <p>{status}</p>
                        {r.research.documents.into_iter().map(|d| {
                            let https = d.citation.url.starts_with("https://");
                            view! {
                                <article>
                                    <p>{d.excerpt}</p>
                                    {https.then(|| view! {
                                        <a class="font-medium text-accent hover:underline" href=d.citation.url.clone() target="_blank" rel="noreferrer">
                                            {d.citation.title.clone()}
                                        </a>
                                    })}
                                </article>
                            }
                        }).collect_view()}
                        <Help text="Research is advisory; your category stays unchanged until you save a review." />
                    </div>
                }
            })}
        </form>
    }
}

/// Run a suggestion pass now; the inbox reloads when it is done.
#[component]
pub fn RequestSuggestions() -> impl IntoView {
    let app = state();
    let busy = RwSignal::new(false);
    let notice = RwSignal::new(String::new());
    let request = move |_| {
        busy.set(true);
        notice.set("Requesting local suggestions…".into());
        spawn_local(async move {
            match invoke::<SuggestionPass>("suggest_categories", &NoArgs {}).await {
                Ok(_) => {
                    notice.set("Suggestions refreshed.".into());
                    app.refresh();
                }
                Err(e) => notice.set(e),
            }
            busy.set(false);
        });
    };
    view! {
        <div class=ACTION_BAR>
            <Button variant=Variant::Secondary disabled=Signal::derive(move || busy.get()) on_click=Callback::new(request)>
                "Request local category suggestions"
            </Button>
            {move || {
                let n = notice.get();
                (!n.is_empty()).then(|| view! { <p role="status" class=MUTED>{n}</p> })
            }}
        </div>
    }
}
