use std::cmp::Ordering;

use leptos::prelude::*;

use super::Fmt;
use crate::ds::patterns::{SORT_BUTTON, TABLE_ROW_HEAD, TABLE_TOTAL_ROW};
use crate::ds::{Icon, Lucide, TABLE, TABLE_CONTAINER, TCELL_NUM, TH, TH_NUM, TROW, cn};

/// A table row: its name, then one value per value column.
pub type TableRow = (String, Vec<Option<f64>>);

/// `rows` ordered by value column `column` (None: as given). Rows with no
/// value there go last either way; ties keep their order.
pub fn sorted_rows(rows: &[TableRow], column: Option<usize>, descending: bool) -> Vec<TableRow> {
    let mut out = rows.to_vec();
    if let Some(c) = column {
        let key = |r: &TableRow| r.1.get(c).copied().flatten().filter(|v| !v.is_nan());
        out.sort_by(|a, b| match (key(a), key(b)) {
            (Some(x), Some(y)) if descending => y.total_cmp(&x),
            (Some(x), Some(y)) => x.total_cmp(&y),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        });
    }
    out
}

/// A heading click: largest first, then reversed, then the original order.
fn next_sort(now: Option<(usize, bool)>, column: usize) -> Option<(usize, bool)> {
    match now {
        Some((c, true)) if c == column => Some((c, false)),
        Some((c, false)) if c == column => None,
        _ => Some((column, true)),
    }
}

/// Values as a table: the first column names each row, then one column per
/// value, sortable by a click on its heading; a totals row when given.
#[component]
pub fn DataTable(
    /// Headings: the name column's, then one per value column.
    columns: Vec<String>,
    rows: Vec<TableRow>,
    totals: Option<Vec<Option<f64>>>,
    format: Fmt,
    /// Per value column; a column without one uses `format`.
    #[prop(optional)]
    formats: Vec<Fmt>,
    #[prop(optional, into)] label: Signal<String>,
) -> impl IntoView {
    let sort = RwSignal::new(None::<(usize, bool)>);
    let fmts: Vec<Fmt> = (0..columns.len().saturating_sub(1))
        .map(|c| formats.get(c).cloned().unwrap_or_else(|| format.clone()))
        .collect();
    let cells = {
        let fmts = fmts.clone();
        move |values: &[Option<f64>]| {
            fmts.iter()
                .enumerate()
                .map(|(c, f)| {
                    let text = values
                        .get(c)
                        .copied()
                        .flatten()
                        .map_or_else(|| "–".to_owned(), |v| f(v));
                    view! { <td class=TCELL_NUM>{text}</td> }
                })
                .collect_view()
        }
    };
    let mut names = columns.into_iter();
    let first = names.next().unwrap_or_default();
    let heads = names
        .enumerate()
        .map(|(c, name)| {
            let aria = move || match sort.get() {
                Some((k, true)) if k == c => "descending",
                Some((k, false)) if k == c => "ascending",
                _ => "none",
            };
            let icon = move || match sort.get() {
                Some((k, true)) if k == c => Some(Icon::ArrowDown),
                Some((k, false)) if k == c => Some(Icon::ArrowUp),
                _ => None,
            };
            view! {
                <th scope="col" class=TH_NUM aria-sort=aria>
                    <button type="button" class=SORT_BUTTON on:click=move |_| sort.update(|s| *s = next_sort(*s, c))>
                        {move || icon().map(|i| view! { <Lucide icon=i size=12 /> })}
                        {name}
                    </button>
                </th>
            }
        })
        .collect_view();
    let body = {
        let cells = cells.clone();
        move || {
            let (column, descending) = sort.get().map_or((None, true), |(c, d)| (Some(c), d));
            sorted_rows(&rows, column, descending)
                .into_iter()
                .map(|(name, values)| {
                    view! {
                        <tr class=TROW>
                            <th scope="row" class=TABLE_ROW_HEAD>{name}</th>
                            {cells(&values)}
                        </tr>
                    }
                })
                .collect_view()
        }
    };
    let foot = totals.map(|t| {
        view! {
            <tfoot>
                <tr class=cn([TROW, TABLE_TOTAL_ROW])>
                    <th scope="row" class=TABLE_ROW_HEAD>"Total"</th>
                    {cells(&t)}
                </tr>
            </tfoot>
        }
    });
    view! {
        <div tabindex="0" role="region" aria-label=move || label.get() class=TABLE_CONTAINER>
            <table class=TABLE data-density="sm">
                <thead>
                    <tr>
                        <th scope="col" class=cn([TH, "text-left"])>{first}</th>
                        {heads}
                    </tr>
                </thead>
                <tbody>{body}</tbody>
                {foot}
            </table>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::{TableRow, next_sort, sorted_rows};

    fn rows() -> Vec<TableRow> {
        vec![
            ("a".into(), vec![Some(2.0), None]),
            ("b".into(), vec![None, Some(1.0)]),
            ("c".into(), vec![Some(5.0), Some(3.0)]),
            ("d".into(), vec![Some(-1.0), Some(2.0)]),
        ]
    }

    fn names(rows: &[TableRow]) -> Vec<&str> {
        rows.iter().map(|r| r.0.as_str()).collect()
    }

    #[test]
    fn sorted_rows_orders_by_a_column_and_puts_missing_last() {
        assert_eq!(
            names(&sorted_rows(&rows(), Some(0), true)),
            ["c", "a", "d", "b"]
        );
        assert_eq!(
            names(&sorted_rows(&rows(), Some(0), false)),
            ["d", "a", "c", "b"]
        );
        assert_eq!(
            names(&sorted_rows(&rows(), Some(1), true)),
            ["c", "d", "b", "a"]
        );
        assert_eq!(
            names(&sorted_rows(&rows(), Some(1), false)),
            ["b", "d", "c", "a"]
        );
    }

    #[test]
    fn sorted_rows_without_a_column_keeps_the_original_order() {
        assert_eq!(
            names(&sorted_rows(&rows(), None, true)),
            ["a", "b", "c", "d"]
        );
        assert_eq!(
            names(&sorted_rows(&rows(), None, false)),
            ["a", "b", "c", "d"]
        );
    }

    #[test]
    fn a_heading_sorts_largest_first_then_reversed_then_unsorted() {
        let first = next_sort(None, 1);
        assert_eq!(first, Some((1, true)));
        assert_eq!(next_sort(first, 1), Some((1, false)));
        assert_eq!(next_sort(Some((1, false)), 1), None);
        assert_eq!(next_sort(Some((0, false)), 1), Some((1, true)));
    }
}
