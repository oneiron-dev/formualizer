use crate::common::build_workbook;
use crate::common::saved_filters::{cases, filtered_xlsx};
use formualizer_eval::engine::ingest::EngineLoadStream;
use formualizer_eval::engine::{Engine, EvalConfig, RowVisibilitySource};
use formualizer_workbook::{
    CalamineAdapter, LiteralValue, LoadStrategy, SpreadsheetReader, Workbook, WorkbookConfig,
};

#[test]
fn calamine_hidden_rows_load_as_manually_hidden() {
    let path = build_workbook(|book| {
        let sheet = book.get_sheet_by_name_mut("Sheet1").unwrap();
        sheet.get_cell_mut((1, 1)).set_value_number(1.0);
        sheet.get_row_dimension_mut(&3).set_hidden(true);
        sheet.get_row_dimension_mut(&4).set_hidden(true);
    });

    let mut adapter = CalamineAdapter::open_path(&path).expect("open xlsx");
    let sheet = adapter.read_sheet("Sheet1").expect("read sheet");

    assert_eq!(sheet.row_hidden_manual, vec![3, 4]);
    assert!(sheet.row_hidden_filter.is_empty());

    let ctx = formualizer_eval::test_workbook::TestWorkbook::new();
    let mut engine: Engine<_> = Engine::new(ctx, EvalConfig::default());
    adapter
        .stream_into_engine(&mut engine)
        .expect("stream into engine");

    assert_eq!(
        engine.is_row_hidden("Sheet1", 3, Some(RowVisibilitySource::Manual)),
        Some(true)
    );
    assert_eq!(
        engine.is_row_hidden("Sheet1", 3, Some(RowVisibilitySource::Filter)),
        Some(false)
    );
    assert_eq!(
        engine.is_row_hidden("Sheet1", 2, Some(RowVisibilitySource::Manual)),
        Some(false)
    );
}

fn evaluate_row_9(bytes: Vec<u8>) -> Vec<f64> {
    let adapter = CalamineAdapter::open_bytes(bytes).expect("open xlsx");
    let mut wb =
        Workbook::from_reader(adapter, LoadStrategy::EagerAll, WorkbookConfig::ephemeral())
            .expect("load workbook");
    wb.evaluate_all().expect("evaluate");
    (1..=4)
        .map(|col| match wb.get_value("Data", 9, col) {
            Some(LiteralValue::Number(n)) => n,
            Some(LiteralValue::Int(i)) => i as f64,
            other => panic!("expected a number in column {col}, got {other:?}"),
        })
        .collect()
}

#[test]
fn calamine_autofilter_hidden_rows_are_skipped_by_every_subtotal() {
    let bytes = filtered_xlsx(
        r#"<sheetPr filterMode="1"/>"#,
        r#"<autoFilter ref="A1:A5"><filterColumn colId="0"><filters><filter val="10"/><filter val="40"/></filters></filterColumn></autoFilter>"#,
    );

    let mut adapter = CalamineAdapter::open_bytes(bytes.clone()).expect("open xlsx");
    let sheet = adapter.read_sheet("Data").expect("read sheet");
    assert_eq!(sheet.row_hidden_filter, vec![3, 4]);
    assert_eq!(sheet.row_hidden_manual, vec![7]);

    // SUBTOTAL(9) keeps the manually hidden 100 but not the filtered 20 and
    // 30; SUBTOTAL(109) drops all three; SUM ignores visibility.
    assert_eq!(evaluate_row_9(bytes), vec![150.0, 50.0, 4.0, 200.0]);
}

#[test]
fn calamine_hidden_rows_under_an_unfiltered_autofilter_stay_manual() {
    let bytes = filtered_xlsx("", r#"<autoFilter ref="A1:A5"/>"#);
    assert_eq!(evaluate_row_9(bytes), vec![200.0, 50.0, 6.0, 200.0]);
}

/// The umya backend asserts the same split for the same packages
/// (tests/umya/row_visibility.rs).
#[test]
fn calamine_splits_saved_hidden_rows_by_filter() {
    for case in cases() {
        let mut adapter = CalamineAdapter::open_bytes(case.bytes.clone()).expect(case.name);
        let sheet = adapter.read_sheet("Data").expect(case.name);
        assert_eq!(sheet.row_hidden_filter, case.filter, "{}", case.name);
        assert_eq!(sheet.row_hidden_manual, case.manual, "{}", case.name);
        assert_eq!(evaluate_row_9(case.bytes), case.row_9, "{}", case.name);
    }
}
