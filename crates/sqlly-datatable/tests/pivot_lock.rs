#![allow(clippy::expect_used)]

use gpui::TestAppContext;
use sqlly_datatable::{GridData, GridTab, PivotConfig, SqllyDataTable};

#[gpui::test]
fn locked_pivot_stays_visible_but_rejects_activation(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_window, cx| {
        let data = GridData::new(vec![], vec![]).expect("empty grid");
        SqllyDataTable::builder(data)
            .pivot(PivotConfig::default())
            .build(cx)
    });

    view.update(cx, |table, cx| {
        assert!(table.pivot_state().is_some(), "Pivot tab remains installed");
        table.set_pivot_locked(true, Some("Loading".to_string()));
        table.set_active_tab(GridTab::Pivot, cx);
        assert!(table.pivot_locked());
        assert_eq!(table.active_tab(), GridTab::Grid);

        table.set_pivot_locked(false, None);
        table.set_active_tab(GridTab::Pivot, cx);
        assert!(!table.pivot_locked());
        assert_eq!(table.active_tab(), GridTab::Pivot);
    });
}
