//! This and next month's plan per currency (`owner-monthly-overview.tsx`).

use finance_types::report::Currency;
use finance_types::report::allocation::CurrencyAllocation;
use finance_types::report::trends::{ForecastMethod, SpendingForecast};
use leptos::ev;
use leptos::prelude::*;
use rust_decimal::Decimal;

use crate::ds::charts::{BudgetBar, fmt};
use crate::ds::{
    Card, CardTitle, CardTone, EmptyState, Eyebrow, Heading, Icon, Lucide, Money, Size, TABLE,
    TABLE_CONTAINER, TCELL, TH, TROW, Variant, button_class, cn, money,
};
use crate::pages::{f, with_report};
use crate::{SpendingSection, Tab, state};

use super::{month_name, remaining_days};

#[component]
pub fn OwnerMonthlyOverview() -> impl IntoView {
    with_report(|report| {
        let Some(allocation) = report
            .monthly_allocation
            .as_ref()
            .filter(|a| !a.currencies.is_empty())
        else {
            return view! {
                <Card tone=CardTone::Muted class="mt-3.5 flex items-center gap-3">
                    <Lucide icon=Icon::ShieldCheck size=20 class="text-fg-muted" />
                    <div>
                        <h2 class="text-base font-semibold">"Monthly plan is waiting for account data."</h2>
                        <p class="mt-1 text-xs text-fg-muted">"Refresh the banks and add at least one spending group limit."</p>
                    </div>
                </Card>
            }
            .into_any();
        };
        let show_forecast = report.monthly_budgets.is_empty();
        let through = allocation.through_date.clone();
        allocation
            .currencies
            .iter()
            .map(|a| {
                let forecasts: Vec<SpendingForecast> = report
                    .spending_forecasts
                    .iter()
                    .filter(|f| f.currency.code == a.currency.code)
                    .cloned()
                    .collect();
                view! {
                    <CurrencyOverview
                        allocation=a.clone()
                        forecasts=forecasts
                        through_date=through.clone()
                        show_forecast=show_forecast
                    />
                }
            })
            .collect_view()
            .into_any()
    })
}

/// The hover/focus tooltip behind the spendable headline.
#[component]
fn BankBalanceBreakdown(allocation: CurrencyAllocation) -> impl IntoView {
    let open = RwSignal::new(false);
    let unit = allocation.currency.clone();
    let m = move |v: Decimal| money(v, &unit);
    let headline = m(allocation.current_wiggle_room);
    let lines: Vec<(&str, Decimal, bool)> = vec![
        ("Included bank balances", allocation.bank_balance, false),
        ("Protected savings", allocation.protected_savings, true),
        ("Salary set aside", allocation.salary_reserved, true),
        ("Before commitments", allocation.current_month_funds, false),
        ("Remaining bills", allocation.current_fixed_remaining, true),
        (
            "Plans, including reservations",
            allocation.current_plans_remaining,
            true,
        ),
        (
            "Remaining group budgets",
            allocation.current_groups_remaining,
            true,
        ),
    ];
    let lines: Vec<(String, String)> = lines
        .into_iter()
        .map(|(label, value, deducted)| {
            (
                label.to_owned(),
                format!("{}{}", if deducted { "− " } else { "" }, m(value)),
            )
        })
        .collect();
    let banks: Vec<(String, bool, String, String, String)> = allocation
        .banks
        .iter()
        .map(|b| {
            (
                b.name.clone(),
                b.included,
                m(b.booked),
                m(b.reserved),
                m(b.available),
            )
        })
        .collect();
    let id = format!("bank-breakdown-{}", allocation.currency.code);
    let tooltip = StoredValue::new((lines, banks, headline.clone(), id.clone()));
    view! {
        <div
            class="relative w-fit"
            on:mouseenter=move |_| open.set(true)
            on:mouseleave=move |e: ev::MouseEvent| {
                let inside = document()
                    .active_element()
                    .zip(e.current_target())
                    .is_some_and(|(active, target)| {
                        use wasm_bindgen::JsCast;
                        target.dyn_into::<web_sys::Node>().is_ok_and(|n| n.contains(Some(&active)))
                    });
                if !inside {
                    open.set(false);
                }
            }
            on:focusout=move |e: ev::FocusEvent| {
                use wasm_bindgen::JsCast;
                let still_inside = e
                    .current_target()
                    .and_then(|t| t.dyn_into::<web_sys::Node>().ok())
                    .zip(e.related_target().and_then(|t| t.dyn_into::<web_sys::Node>().ok()))
                    .is_some_and(|(container, next)| container.contains(Some(&next)));
                if !still_inside {
                    open.set(false);
                }
            }
            on:keydown=move |e: ev::KeyboardEvent| {
                if e.key() == "Escape" {
                    open.set(false);
                }
            }
        >
            <h2 class="mt-1 text-2xl font-semibold tracking-tight sm:text-display">
                <button
                    type="button"
                    class=cn([button_class(Variant::Plain, Size::Inline).as_str(), "text-left focus-visible:outline-offset-4"])
                    aria-label=format!("Spendable {headline} · Show bank balance breakdown")
                    aria-expanded=move || open.get().to_string()
                    aria-describedby=move || open.get().then(|| id.clone())
                    on:focus=move |_| open.set(true)
                    on:click=move |_| open.set(true)
                >
                    <span class="tabular-nums">{headline.clone()}</span>
                    <Lucide icon=Icon::Info size=15 class="text-fg-muted" />
                </button>
            </h2>
            <Show when=move || open.get()>
                {move || {
                    let (lines, banks, headline, tip_id) = tooltip.get_value();
                    view! {
                        <div
                            id=tip_id
                            role="tooltip"
                            class="absolute left-0 top-full z-30 mt-2 max-h-[75vh] w-[min(570px,calc(100vw-7.5rem))] overflow-auto rounded-lg border border-border bg-card-strong p-3 text-left text-xs font-normal tracking-normal text-fg shadow-pop"
                        >
                            <strong class="block text-sm font-semibold">"Bank balance breakdown"</strong>
                            <small class="mb-3 mt-0.5 block text-fg-muted">"Recorded balances · reservations shown separately"</small>
                            {if banks.is_empty() {
                                view! { <p class="text-fg-muted">"Account breakdown unavailable. Refresh the report."</p> }.into_any()
                            } else {
                                view! {
                                    <table class=cn([TABLE, "text-xs"]) data-density="sm">
                                        <thead>
                                            <tr>
                                                <th class=cn([TH, "text-left px-1.5"])>"Bank"</th>
                                                <th class=cn([TH, "text-right tabular-nums px-1.5"])>"Booked"</th>
                                                <th class=cn([TH, "text-right tabular-nums px-1.5"])>"Reserved"</th>
                                                <th class=cn([TH, "text-right tabular-nums px-1.5"])>"Available"</th>
                                            </tr>
                                        </thead>
                                        <tbody>
                                            {banks
                                                .into_iter()
                                                .map(|(name, included, booked, reserved, available)| {
                                                    let cell = cn([TCELL, "text-right tabular-nums whitespace-nowrap px-1.5"]);
                                                    view! {
                                                        <tr class=cn([TROW, if included { "" } else { "text-fg-muted" }])>
                                                            <th scope="row" class="px-1.5 py-1.5 text-left align-top font-medium">
                                                                {name}
                                                                {(!included).then(|| view! { <small class="block font-normal text-fg-muted">"Excluded from total"</small> })}
                                                            </th>
                                                            <td class=cell.clone()>{booked}</td>
                                                            <td class=cell.clone()>{reserved}</td>
                                                            <td class=cell>{available}</td>
                                                        </tr>
                                                    }
                                                })
                                                .collect_view()}
                                        </tbody>
                                    </table>
                                }
                                .into_any()
                            }}
                            <dl class="mt-3.5 grid gap-1.5">
                                {lines
                                    .into_iter()
                                    .map(|(label, value)| {
                                        view! {
                                            <div class="flex justify-between gap-4">
                                                <dt>{label}</dt>
                                                <dd class="whitespace-nowrap tabular-nums">{value}</dd>
                                            </div>
                                        }
                                    })
                                    .collect_view()}
                                <div class="flex justify-between gap-4 border-t border-line pt-2 font-semibold">
                                    <dt>"Spendable"</dt>
                                    <dd class="whitespace-nowrap tabular-nums">{headline}</dd>
                                </div>
                            </dl>
                        </div>
                    }
                }}
            </Show>
        </div>
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Fixed,
    Groups,
    Goals,
    Plans,
    Free,
}

impl Part {
    fn fill(self) -> &'static str {
        match self {
            Self::Fixed => "bg-chart-fixed",
            Self::Groups => "bg-chart-1",
            Self::Goals => "bg-chart-6",
            Self::Plans => "bg-chart-5",
            Self::Free => "bg-chart-free",
        }
    }
}

/// `toFixed(2)` of a value the site parsed with `Number`.
fn fixed2(value: Decimal) -> Decimal {
    value.round_dp(2)
}

fn allocation_bar(total: Decimal, parts: &[(String, Decimal, Part)], unit: &Currency) -> AnyView {
    let sum: Decimal = parts.iter().map(|p| p.1).sum();
    let scale = f(total.max(sum).max(Decimal::ONE));
    let label = parts
        .iter()
        .map(|(l, v, _)| format!("{l} {}", money(*v, unit)))
        .collect::<Vec<_>>()
        .join(", ");
    let pieces = parts
        .iter()
        .filter(|p| p.1 > Decimal::ZERO)
        .map(|(l, v, tone)| {
            let width = (f(*v) / scale * 100.0).clamp(0.0, 100.0);
            view! {
                <i
                    class=cn(["block h-full min-w-0.5 border-r-2 border-card-strong last:border-r-0", tone.fill()])
                    style:width=format!("{width}%")
                    title=format!("{l}: {}", money(*v, unit))
                ></i>
            }
        })
        .collect_view();
    view! {
        <div role="img" class="mb-3 mt-4 flex h-3 w-full overflow-hidden rounded-pill bg-soft" aria-label=label>
            {pieces}
        </div>
    }
    .into_any()
}

fn allocation_legend(parts: &[(String, Decimal, Part)], unit: &Currency) -> AnyView {
    let rows = parts
        .iter()
        .map(|(label, value, tone)| {
            let value = fixed2(*value);
            view! {
                <span
                    tabindex="0"
                    aria-label=format!("{label}: {}", money(value, unit))
                    class="grid min-w-0 grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-2 rounded-sm text-xs text-fg-muted focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
                >
                    <i aria-hidden="true" class=cn(["size-2 rounded-pill", tone.fill()])></i>
                    <b class="truncate font-normal">{label.clone()}</b>
                    <Money value=value unit=unit.clone() neutral=true class="truncate font-medium text-fg" />
                </span>
            }
        })
        .collect_view();
    view! { <div class="grid grid-cols-1 gap-x-3 gap-y-1.5 sm:grid-cols-2">{rows}</div> }.into_any()
}

fn allocation_result(label: &str, value: Decimal, unit: &Currency, negative: bool) -> AnyView {
    view! {
        <div class="mt-3 flex items-baseline justify-between gap-4 border-t border-line pt-3 text-sm text-fg-muted">
            <span>{label.to_owned()}</span>
            <strong class=cn([
                "text-base font-semibold tabular-nums",
                if negative { "text-negative-fg" } else { "text-positive-fg" },
            ])>{money(value, unit)}</strong>
        </div>
    }
    .into_any()
}

fn monthly_status(warning: bool, title: String, detail: String) -> impl IntoView {
    view! {
        <div class=cn([
            "grid grid-cols-[auto_minmax(0,1fr)] items-center gap-x-3 gap-y-1 rounded-xl border px-4 py-3 text-sm text-fg sm:flex sm:flex-wrap",
            if warning { "border-warning/30 bg-warning/10" } else { "border-positive/30 bg-positive/10" },
        ])>
            <span class=cn(["shrink-0", if warning { "text-warning-fg" } else { "text-positive-fg" }])>
                <Lucide icon=if warning { Icon::CircleAlert } else { Icon::ShieldCheck } size=18 />
            </span>
            <strong class="font-semibold">{title}</strong>
            <span class="col-start-2 text-fg-muted">{detail}</span>
        </div>
    }
    .into_any()
}

fn forecast_row(
    forecast: SpendingForecast,
    allocation: &CurrencyAllocation,
    remaining_days: u32,
) -> AnyView {
    let limit = allocation
        .groups
        .iter()
        .find(|g| g.group == forecast.group)
        .map_or(Decimal::ZERO, |g| g.limit);
    let unit = forecast.currency.clone();
    let has_limit = limit > Decimal::ZERO;
    let over = has_limit && forecast.expected > limit;
    let daily_room = if has_limit {
        ((limit - forecast.spent).max(Decimal::ZERO) / Decimal::from(remaining_days.max(1)))
            .round_dp(2)
    } else {
        Decimal::ZERO
    };
    let method = if forecast.method == ForecastMethod::CatboostQuantile {
        "Forecast"
    } else {
        "History + pace"
    };
    let bar_unit = unit.clone();
    let bar = if has_limit {
        view! {
            <BudgetBar
                label=format!("{} spending", forecast.group)
                spent=f(forecast.spent)
                limit=f(limit)
                forecast=f(forecast.expected)
                format=fmt(move |n| money(Decimal::try_from(n).unwrap_or_default(), &bar_unit))
                hide_label=true
            />
        }
        .into_any()
    } else {
        view! {
            <p class="text-xs tabular-nums text-fg-muted">
                {format!("{} spent · forecast {}", money(forecast.spent, &unit), money(forecast.expected, &unit))}
            </p>
        }
        .into_any()
    };
    let action = if has_limit {
        view! {
            <strong class=cn(["block text-sm font-semibold", if over { "text-negative-fg" } else { "" }])>
                {if over { "Slow down" } else { "On track" }}
            </strong>
            <small class="block text-xs text-fg-muted">{format!("{}/day left", money(daily_room, &unit))}</small>
        }
        .into_any()
    } else {
        view! {
            <strong class="block text-sm font-semibold">"No limit"</strong>
            <small class="block text-xs text-fg-muted">"Set in Spending"</small>
        }
        .into_any()
    };
    view! {
        <tr class=cn([TROW, "align-middle"])>
            <th scope="row" class="px-2 py-2.5 text-left align-middle">
                <strong class="block text-sm font-semibold">{forecast.group.clone()}</strong>
                <small class="block text-xs font-normal text-fg-muted">{method}</small>
            </th>
            <td class=cn([TCELL, "hidden align-middle md:table-cell"])>{bar}</td>
            <td class=cn([TCELL, "align-middle"])>
                <strong class="block text-sm font-semibold tabular-nums">{money(forecast.expected, &unit)}</strong>
                <small class="block text-xs tabular-nums text-fg-muted">
                    {format!("{}–{}", money(forecast.lower, &unit), money(forecast.upper, &unit))}
                </small>
            </td>
            <td class=cn([TCELL, "hidden align-middle sm:table-cell"])>{action}</td>
        </tr>
    }
    .into_any()
}

#[component]
fn CurrencyOverview(
    allocation: CurrencyAllocation,
    forecasts: Vec<SpendingForecast>,
    through_date: String,
    show_forecast: bool,
) -> impl IntoView {
    let app = state();
    let a = &allocation;
    let unit = a.currency.clone();
    let zero = Decimal::ZERO;
    let current_total = a.current_month_funds.max(zero);
    let next_total = a.next_planned_funds.max(zero);
    let next_funded = next_total > zero;
    let next_negative = next_funded && a.next_shortfall > zero;
    let remaining = remaining_days(&through_date);

    let mut current_parts = vec![("Bills".to_owned(), a.current_fixed_remaining, Part::Fixed)];
    current_parts.extend(
        a.groups
            .iter()
            .map(|g| (g.group.clone(), g.remaining, Part::Groups)),
    );
    current_parts.push(("Plans".into(), a.current_plans_remaining, Part::Goals));
    current_parts.push((
        "Wiggle room".into(),
        a.current_wiggle_room.max(zero),
        Part::Free,
    ));

    let mut next_parts = vec![("Bills".to_owned(), a.next_fixed, Part::Fixed)];
    if a.next_group_rows.is_empty() {
        next_parts.extend(
            a.groups
                .iter()
                .map(|g| (g.group.clone(), g.limit, Part::Groups)),
        );
    } else {
        next_parts.extend(
            a.next_group_rows
                .iter()
                .map(|g| (g.group.clone(), g.remaining, Part::Groups)),
        );
    }
    if a.next_goals > zero {
        next_parts.push(("Savings goals".into(), a.next_goals, Part::Goals));
    }
    next_parts.push(("Other plans".into(), a.next_plans, Part::Plans));
    next_parts.push(("Unallocated".into(), a.next_wiggle_room, Part::Free));

    let limit_of = |group: &str| {
        a.groups
            .iter()
            .find(|g| g.group == group)
            .map_or(zero, |g| g.limit)
    };
    let mut ordered = forecasts;
    ordered.sort_by(|l, r| {
        let (ll, rl) = (limit_of(&l.group), limit_of(&r.group));
        if (ll > zero) != (rl > zero) {
            return if rl > zero {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Less
            };
        }
        let lr = l.expected / ll.max(Decimal::ONE);
        let rr = r.expected / rl.max(Decimal::ONE);
        rr.cmp(&lr)
    });

    let status = if a.salary_at_risk > zero || next_negative {
        let title = if a.salary_at_risk > zero {
            format!(
                "{} of next month’s salary is at risk.",
                money(a.salary_at_risk, &unit)
            )
        } else {
            format!(
                "{} is missing from next month’s plan.",
                money(a.next_shortfall, &unit)
            )
        };
        monthly_status(
            true,
            title,
            "Lower a variable group or adjust the plan.".into(),
        )
        .into_any()
    } else if !next_funded {
        monthly_status(
            false,
            "This month stays separate.".into(),
            "Next month’s plan will lock when salary arrives.".into(),
        )
        .into_any()
    } else {
        monthly_status(
            false,
            "Next month’s plan is covered by booked and expected income.".into(),
            format!(
                "{} remains after this month’s plan.",
                money(a.current_wiggle_room, &unit)
            ),
        )
        .into_any()
    };

    let expected_note = (a.current_expected_income > zero).then(|| {
        view! {
            <p class="mt-3 text-xs leading-relaxed text-fg-muted">
                {format!(
                    "With {} expected extra income: {} projected after commitments.",
                    money(a.current_expected_income, &unit),
                    money(a.current_planned_wiggle_room, &unit),
                )}
            </p>
        }
    });
    let next_label = if !next_funded {
        "Planned before salary"
    } else if a.next_shortfall > zero {
        "Plan shortfall"
    } else {
        "Still unallocated"
    };
    let next_value = if !next_funded || a.next_shortfall > zero {
        a.next_shortfall
    } else {
        a.next_wiggle_room
    };

    let forecast_card = show_forecast.then(|| {
        let table = if ordered.is_empty() {
            view! {
                <EmptyState class="mt-3 p-5" icon=Icon::TrendingDown title="Add group limits to see month-end forecasts." />
            }
            .into_any()
        } else {
            let rows = ordered
                .into_iter()
                .map(|fc| forecast_row(fc, &allocation, remaining))
                .collect_view();
            view! {
                <div aria-label="Variable spending forecast" tabindex="0" class=cn([TABLE_CONTAINER, "mt-3 max-h-96 overflow-y-auto"])>
                    <table class=cn([TABLE, "table-fixed"])>
                        <thead>
                            <tr>
                                <th class=cn([TH, "text-left w-[45%] sm:w-[34%] md:w-[18%]"])>"Group"</th>
                                <th class=cn([TH, "text-left hidden w-[42%] md:table-cell"])>"Spent · forecast · limit"</th>
                                <th class=cn([TH, "text-left w-[55%] sm:w-[33%] md:w-[20%]"])>"Month end"</th>
                                <th class=cn([TH, "text-left hidden w-[33%] sm:table-cell md:w-[20%]"])>"Action"</th>
                            </tr>
                        </thead>
                        <tbody>{rows}</tbody>
                    </table>
                </div>
            }
            .into_any()
        };
        view! {
            <Card class="min-w-0">
                <header class="flex flex-wrap items-start justify-between gap-x-4 gap-y-1">
                    <div class="min-w-0">
                        <Eyebrow level=Heading::P>"VARIABLE SPENDING FORECAST"</Eyebrow>
                        <CardTitle level=Heading::H2 class="mt-1">"Where this month is heading"</CardTitle>
                    </div>
                    <button
                        type="button"
                        class=cn([button_class(Variant::Ghost, Size::Sm).as_str(), "shrink-0 text-accent"])
                        on:click=move |_| app.go_spending(SpendingSection::PlanPatterns)
                    >
                        "Adjust limits "
                        <Lucide icon=Icon::ArrowRight size=14 />
                    </button>
                </header>
                {table}
                <footer class="mt-3 flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-fg-muted">
                    <span class="flex items-center gap-1.5" tabindex="0" aria-label="Spent">
                        <i aria-hidden="true" class="h-1.5 w-2.5 rounded-pill bg-chart-1"></i>
                        <b class="font-normal">"spent"</b>
                    </span>
                    <span class="flex items-center gap-1.5" tabindex="0" aria-label="Forecast">
                        <i aria-hidden="true" class="h-1.5 w-2.5 rounded-pill bg-chart-1/30"></i>
                        <b class="font-normal">"forecast"</b>
                    </span>
                    <span class="flex items-center gap-1.5" tabindex="0" aria-label="Limit">
                        <i aria-hidden="true" class="h-2.5 w-0.5 rounded-pill bg-chart-limit"></i>
                        <b class="font-normal">"limit"</b>
                    </span>
                    <small class="ml-auto text-xs">{format!("{remaining} days left")}</small>
                </footer>
            </Card>
        }
    });

    let section_label = format!("{} monthly plan", unit.code);
    let this_month = format!("THIS MONTH · {}", month_name(&a.current_month));
    let funds = format!("FUNDS {}", month_name(&a.funded_month));
    let bank = format!("Bank {}", money(a.bank_balance, &unit));
    let savings = format!("− savings {}", money(a.protected_savings, &unit));
    let salary = format!("− salary {}", money(a.salary_reserved, &unit));
    let next_funds = money(a.next_planned_funds, &unit);
    let next_income = format!(
        "Salary + {} expected extra income",
        money(a.next_expected_income, &unit)
    );
    let current_bar = allocation_bar(current_total, &current_parts, &unit);
    let current_legend = allocation_legend(&current_parts, &unit);
    let current_result =
        allocation_result("Spendable wiggle room", a.current_wiggle_room, &unit, false);
    let next_bar = allocation_bar(next_total, &next_parts, &unit);
    let next_legend = allocation_legend(&next_parts, &unit);
    let next_result = allocation_result(next_label, next_value, &unit, next_negative);
    let breakdown = view! { <BankBalanceBreakdown allocation=allocation.clone() /> }.into_any();
    view! {
        <section class="mt-3.5 flex flex-col gap-3.5 text-fg" aria-label=section_label>
            {status}
            <div class="grid grid-cols-1 gap-3.5 md:grid-cols-2">
                <Card class="min-w-0">
                    <header class="flex items-start justify-between gap-4">
                        <div class="min-w-0">
                            <Eyebrow level=Heading::P>{this_month}</Eyebrow>
                            {breakdown}
                            <small class="text-xs text-fg-muted">"spendable after remaining commitments"</small>
                        </div>
                        <div class="hidden gap-0.5 text-right text-xs tabular-nums text-fg-muted sm:grid">
                            <span>{bank}</span>
                            <span>{savings}</span>
                            <span>{salary}</span>
                        </div>
                    </header>
                    {current_bar}
                    {current_legend}
                    {current_result}
                    {expected_note}
                </Card>
                <Card class="min-w-0">
                    <header class="flex items-start justify-between gap-4">
                        <div class="min-w-0">
                            <Eyebrow level=Heading::P>{funds}</Eyebrow>
                            <h2 class="mt-1 text-2xl font-semibold tracking-tight tabular-nums sm:text-display">
                                {next_funds}
                            </h2>
                            <small class="text-xs text-fg-muted">{next_income}</small>
                        </div>
                        <button
                            type="button"
                            class=cn([button_class(Variant::Ghost, Size::Sm).as_str(), "shrink-0 text-accent"])
                            on:click=move |_| app.go(Tab::Plan)
                        >
                            "Edit plan "
                            <Lucide icon=Icon::ArrowRight size=14 />
                        </button>
                    </header>
                    {next_bar}
                    {next_legend}
                    {next_result}
                </Card>
            </div>
            {forecast_card}
        </section>
    }
}
