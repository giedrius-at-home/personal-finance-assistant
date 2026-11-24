//! The app's ledger and AI commands by name, with their arguments as the UI
//! sends them to Tauri, for the server and the devserver. Commands that need
//! the app or the server (bank sync, email, updates, the forecast model)
//! are each caller's own.

use finance_ai::Assistant;
use finance_core::service::Service;
use finance_types::report::review::TransactionSearchQuery;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct PlanArgs {
    plan: finance_types::gateway::BudgetPlan,
}

#[derive(Deserialize)]
struct PlannedPaymentArgs {
    request: finance_types::gateway::PlannedPaymentRequest,
}

#[derive(Deserialize)]
struct ArchivePlanArgs {
    plan_id: String,
}

#[derive(Deserialize)]
struct MonthArgs {
    month: String,
}

#[derive(Deserialize)]
struct ClosureActionArgs {
    action: finance_types::report::closure::ClosureAction,
}

#[derive(Deserialize)]
struct WishDraftArgs {
    draft: finance_types::gateway::WishDraft,
}

#[derive(Deserialize)]
struct WishSavedArgs {
    wish_id: String,
    saved: rust_decimal::Decimal,
}

#[derive(Deserialize)]
struct MoveWishArgs {
    wish_id: String,
    #[serde(default)]
    event_id: Option<String>,
}

#[derive(Deserialize)]
struct WishIdArgs {
    wish_id: String,
}

#[derive(Deserialize)]
struct EventDatesArgs {
    wish_id: String,
    starts_on: String,
    ends_on: String,
}

#[derive(Deserialize)]
struct EventTransactionArgs {
    wish_id: String,
    transaction_id: String,
    pick: finance_types::gateway::EventPick,
}

#[derive(Deserialize)]
struct WishMustArgs {
    wish_id: String,
    must: bool,
}

#[derive(Deserialize)]
struct WishBoughtArgs {
    wish_id: String,
    purchase: finance_types::gateway::WishPurchase,
}

#[derive(Deserialize)]
struct WishStartArgs {
    wish_id: String,
    #[serde(default)]
    match_category: Option<String>,
}

#[derive(Deserialize)]
struct WishSettingsArgs {
    settings: finance_types::gateway::WishSettings,
}

#[derive(Deserialize)]
struct UiLayoutArgs {
    layout: finance_types::ui::UiLayout,
}

#[derive(Deserialize)]
struct RoleArgs {
    role: finance_types::report::closure::AccountRoleDraft,
}

#[derive(Deserialize)]
struct SearchArgs {
    query: TransactionSearchQuery,
}

#[derive(Deserialize)]
struct GroupArgs {
    category: String,
    group: Option<String>,
    revision: u32,
}

#[derive(Deserialize)]
struct NewGroupArgs {
    group: String,
}

pub fn ok<T: serde::Serialize>(result: Result<T, String>) -> Result<Value, String> {
    result.and_then(|v| serde_json::to_value(v).map_err(|e| e.to_string()))
}

/// The command argument `name`, as Tauri would pass it.
pub fn arg<T: serde::de::DeserializeOwned>(mut args: Value, name: &str) -> Result<T, String> {
    serde_json::from_value(args.get_mut(name).map(Value::take).unwrap_or(Value::Null))
        .map_err(|e| e.to_string())
}

/// The `request` argument of a write command.
fn request<T: serde::de::DeserializeOwned>(args: Value) -> Result<T, String> {
    arg(args, "request")
}

fn parse<T: serde::de::DeserializeOwned>(args: Value) -> Result<T, String> {
    serde_json::from_value(args).map_err(|e| e.to_string())
}

/// Run `cmd`; `None` when it is not one of these.
pub fn run(
    service: &Service,
    assistant: &Assistant,
    cmd: &str,
    args: Value,
) -> Option<Result<Value, String>> {
    Some(match cmd {
        "dashboard" => ok(service.dashboard()),
        "review" => ok(service.review()),
        "status" => ok(service.status()),
        "refresh" => {
            service.refresh();
            // No desktop here: due alerts are recorded as `unavailable`
            // (or `disabled`), never shown.
            let _ = service.evaluate_alerts_now();
            Ok(Value::Null)
        }
        "save_budget_plan" => {
            parse::<PlanArgs>(args).and_then(|a| ok(service.save_budget_plan(a.plan)))
        }
        "save_spending_group" => parse::<GroupArgs>(args).and_then(|a| {
            ok(service.save_spending_group(&a.category, a.group.as_deref(), a.revision))
        }),
        "create_spending_group" => {
            parse::<NewGroupArgs>(args).and_then(|a| ok(service.create_spending_group(&a.group)))
        }
        "create_planned_payment" => parse::<PlannedPaymentArgs>(args)
            .and_then(|a| ok(service.create_planned_payment(&a.request))),
        "archive_planned_payment" => parse::<ArchivePlanArgs>(args)
            .and_then(|a| ok(service.archive_planned_payment(&a.plan_id))),
        "tie_planned_payment" => arg::<String>(args.clone(), "plan_id").and_then(|plan| {
            let transaction = arg::<Option<String>>(args, "transaction_id")?;
            ok(service.tie_planned_payment(&plan, transaction.as_deref()))
        }),
        "search_transactions" => {
            parse::<SearchArgs>(args).and_then(|a| ok(service.search(&a.query)))
        }
        "closure" => parse::<MonthArgs>(args).and_then(|a| ok(service.closure(&a.month))),
        "closure_action" => {
            parse::<ClosureActionArgs>(args).and_then(|a| ok(service.closure_action(a.action)))
        }
        "wishes" => ok(service.wishes()),
        "save_wish" => parse::<WishDraftArgs>(args).and_then(|a| ok(service.save_wish(a.draft))),
        "record_wish_saved" => parse::<WishSavedArgs>(args)
            .and_then(|a| ok(service.record_wish_saved(&a.wish_id, a.saved))),
        "drop_wish" => parse::<WishIdArgs>(args).and_then(|a| ok(service.drop_wish(&a.wish_id))),
        "events" => ok(service.events()),
        "plan_wish_as_event" => parse::<EventDatesArgs>(args)
            .and_then(|a| ok(service.plan_wish_as_event(&a.wish_id, &a.starts_on, &a.ends_on))),
        "wish_back_from_event" => {
            parse::<WishIdArgs>(args).and_then(|a| ok(service.wish_back_from_event(&a.wish_id)))
        }
        "move_wish" => parse::<MoveWishArgs>(args)
            .and_then(|a| ok(service.move_wish(&a.wish_id, a.event_id.as_deref()))),
        "set_event_transaction" => parse::<EventTransactionArgs>(args)
            .and_then(|a| ok(service.set_event_transaction(&a.wish_id, &a.transaction_id, a.pick))),
        "set_wish_must" => {
            parse::<WishMustArgs>(args).and_then(|a| ok(service.set_wish_must(&a.wish_id, a.must)))
        }
        "mark_wish_bought" => parse::<WishBoughtArgs>(args)
            .and_then(|a| ok(service.mark_wish_bought(&a.wish_id, a.purchase))),
        "start_wish" => parse::<WishStartArgs>(args)
            .and_then(|a| ok(service.start_wish(&a.wish_id, a.match_category))),
        "save_wish_settings" => {
            parse::<WishSettingsArgs>(args).and_then(|a| ok(service.save_wish_settings(a.settings)))
        }
        "ui_layout" => ok(service.ui_layout()),
        "save_ui_layout" => {
            parse::<UiLayoutArgs>(args).and_then(|a| ok(service.save_ui_layout(a.layout)))
        }
        "plot_query" => arg(args, "spec").and_then(|spec| ok(service.plot_query(spec))),
        "save_account_role" => {
            parse::<RoleArgs>(args).and_then(|a| ok(service.save_account_role(a.role)))
        }
        "unclassified_accounts" => ok(service.unclassified_accounts()),
        "personal_transactions" => {
            arg::<String>(args, "query").and_then(|q| ok(service.personal_transactions(&q)))
        }
        "save_personal_choice" => {
            arg(args, "choice").and_then(|c| ok(service.save_personal_choice(c)))
        }
        "set_transaction_category" => {
            arg(args, "change").and_then(|c| ok(service.set_transaction_category(c)))
        }
        "review_decisions" => arg(args, "decisions").and_then(|d| ok(service.review_decisions(d))),
        "review_batch" => arg(args, "items").and_then(|i| ok(service.review_batch(i))),
        "save_review_details" => {
            arg(args, "details").and_then(|d| ok(service.save_review_details(d)))
        }
        "audit_daily_review" => {
            arg::<String>(args, "transaction").and_then(|t| ok(service.audit_daily_review(&t)))
        }
        "preview_consolidation" => {
            arg(args, "request").and_then(|r| ok(service.preview_consolidation(&r)))
        }
        "apply_consolidation" => {
            arg(args, "request").and_then(|r| ok(service.apply_consolidation(r)))
        }
        "create_bill" => request(args).and_then(|r| ok(service.create_bill(r))),
        "edit_bill" => request(args).and_then(|r| ok(service.edit_bill(r))),
        "archive_bill" => request(args).and_then(|r| ok(service.archive_bill(r))),
        "save_subscription_categories" => {
            request(args).and_then(|r| ok(service.save_subscription_categories(r)))
        }
        "link_bill_payment" => request(args).and_then(|r| ok(service.link_bill_payment(r))),
        "decline_bill_match" => request(args).and_then(|r| ok(service.decline_bill_match(r))),
        "manage_workspace" => ok(service.manage_workspace()),
        "banks" => ok(service.banks()),
        "reconcile_bank_account" => arg::<String>(args.clone(), "bank").and_then(|bank| {
            ok(service.reconcile_bank_account(&bank, arg::<i64>(args, "account_id")?))
        }),
        "ai_status" => ok(assistant.status(service)),
        "save_ai_settings" => {
            arg(args, "settings").and_then(|s| ok(assistant.save_settings(service, s)))
        }
        "set_typesafe_key" => {
            arg::<String>(args, "key").and_then(|k| ok(assistant.set_typesafe_key(&k)))
        }
        "forget_typesafe_key" => ok(assistant.forget_typesafe_key()),
        "suggest_categories" => ok(assistant.suggest_now(service)),
        "suggest_spending_group" => {
            arg::<String>(args, "category").and_then(|c| ok(assistant.suggest_group(service, &c)))
        }
        "research_merchant" => arg::<String>(args.clone(), "proposal").and_then(|proposal| {
            let public_name = arg::<String>(args, "public_name")?;
            ok(assistant.research_merchant(service, &proposal, &public_name))
        }),
        "preview_csv_import" => arg(args, "upload").and_then(|u| ok(service.preview_csv_import(u))),
        "commit_csv_import" => arg(args, "commit").and_then(|c| ok(service.commit_csv_import(c))),
        "preview_settings" => arg(args, "form").and_then(|f| ok(service.preview_settings(f))),
        "apply_settings" => arg(args, "apply").and_then(|a| ok(service.apply_settings(a))),
        "rollback_settings" => arg(args, "revision").and_then(|r| ok(service.rollback_settings(r))),
        _ => return None,
    })
}
