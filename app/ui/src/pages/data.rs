use finance_types::DataStatus;
use leptos::prelude::*;

use crate::api::{NoArgs, load};
use crate::format;
use crate::widgets::{Problem, Skeleton};

#[component]
pub fn Data() -> impl IntoView {
    let status = load::<DataStatus, _>("status", || NoArgs {});
    move || {
        match status.data.get() {
        None => view! { <Skeleton rows=3 /> }.into_any(),
        Some(Err(e)) => view! { <Problem error=e /> }.into_any(),
        Some(Ok(s)) => {
            view! {
                <section class="card panel">
                    <div class="panel-head">
                        <h2>"Local data"</h2>
                    </div>
                    <dl class="facts wide">
                        <dt>"Stored at"</dt>
                        <dd><code>"~/Library/Application Support/Finance/finance.sqlite3"</code></dd>
                        <dt>"Transactions"</dt>
                        <dd>{format::count(s.transactions)}</dd>
                        <dt>"Covers"</dt>
                        <dd>
                            {match (s.first_date, s.last_date) {
                                (Some(a), Some(b)) => format!("{} – {}", format::date(&a), format::date(&b)),
                                _ => "—".to_owned(),
                            }}
                        </dd>
                        <dt>"Imported"</dt>
                        <dd>
                            {s.imported_at.map(|t| format!("{} {} UTC", format::date(&t), &t[11..16])).unwrap_or_default()}
                        </dd>
                        <dt>"Source"</dt>
                        <dd>{s.source.unwrap_or_default()}</dd>
                    </dl>
                    <p class="muted pad">
                        "Everything stays on this Mac. Bank sync, archiving to Google Drive and updates arrive in later releases."
                    </p>
                </section>
            }
                .into_any()
        }
    }
    }
}
