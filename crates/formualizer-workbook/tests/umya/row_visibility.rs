use crate::common::build_workbook;
use crate::common::saved_filters::{cases, filtered_xlsx};
use formualizer_eval::engine::ingest::EngineLoadStream;
use formualizer_eval::engine::{Engine, EvalConfig, RowVisibilitySource};
use formualizer_workbook::{
    LiteralValue, LoadStrategy, SpreadsheetReader, UmyaAdapter, Workbook, WorkbookConfig,
};

fn assert_num(value: LiteralValue, expected: f64) {
    match value {
        LiteralValue::Number(n) => assert!((n - expected).abs() < 1e-9, "{n} != {expected}"),
        LiteralValue::Int(i) => assert!(((i as f64) - expected).abs() < 1e-9),
        other => panic!("expected numeric {expected}, got {other:?}"),
    }
}

fn op_expected(function_num_1_to_11: i32, values: &[f64]) -> f64 {
    assert!(!values.is_empty());

    let n = values.len() as f64;
    let sum = values.iter().copied().sum::<f64>();
    let mean = sum / n;

    match function_num_1_to_11 {
        1 => mean,
        2 | 3 => n,
        4 => values.iter().copied().reduce(f64::max).unwrap(),
        5 => values.iter().copied().reduce(f64::min).unwrap(),
        6 => values.iter().copied().product::<f64>(),
        7 => {
            let ss = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>();
            (ss / (n - 1.0)).sqrt()
        }
        8 => {
            let ss = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>();
            (ss / n).sqrt()
        }
        9 => sum,
        10 => {
            let ss = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>();
            ss / (n - 1.0)
        }
        11 => {
            let ss = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>();
            ss / n
        }
        _ => panic!("unsupported op code: {function_num_1_to_11}"),
    }
}

#[test]
fn umya_hidden_rows_ingest_as_manual_visibility() {
    let path = build_workbook(|book| {
        let sheet = book.get_sheet_by_name_mut("Sheet1").unwrap();
        sheet.get_cell_mut((1, 1)).set_value_number(123.0);
        sheet.get_row_dimension_mut(&2).set_hidden(true);
        sheet.get_row_dimension_mut(&5).set_hidden(true);
    });

    let mut adapter = UmyaAdapter::open_path(&path).expect("open xlsx");
    let sheet = adapter.read_sheet("Sheet1").expect("read sheet");
    assert_eq!(sheet.row_hidden_manual, vec![2, 5]);
    assert!(sheet.row_hidden_filter.is_empty());

    let ctx = formualizer_eval::test_workbook::TestWorkbook::new();
    let mut engine: Engine<_> = Engine::new(ctx, EvalConfig::default());
    adapter
        .stream_into_engine(&mut engine)
        .expect("stream into engine");

    assert_eq!(
        engine.is_row_hidden("Sheet1", 2, Some(RowVisibilitySource::Manual)),
        Some(true)
    );
    assert_eq!(
        engine.is_row_hidden("Sheet1", 5, Some(RowVisibilitySource::Manual)),
        Some(true)
    );
    assert_eq!(
        engine.is_row_hidden("Sheet1", 2, Some(RowVisibilitySource::Filter)),
        Some(false)
    );
}

#[test]
fn umya_hidden_rows_affect_subtotal_end_to_end() {
    let path = build_workbook(|book| {
        let sheet = book.get_sheet_by_name_mut("Sheet1").unwrap();
        sheet.get_cell_mut((1, 2)).set_value_number(10.0); // A2
        sheet.get_cell_mut((1, 3)).set_value_number(20.0); // A3 (hidden)
        sheet.get_cell_mut((1, 4)).set_value_number(30.0); // A4
        sheet.get_cell_mut((1, 5)).set_value_number(100.0); // A5
        sheet
            .get_cell_mut((2, 1))
            .set_formula("=SUBTOTAL(109,A2:A5)"); // B1
        sheet.get_row_dimension_mut(&3).set_hidden(true);
    });

    let backend = UmyaAdapter::open_path(&path).expect("open xlsx");
    let mut wb =
        Workbook::from_reader(backend, LoadStrategy::EagerAll, WorkbookConfig::ephemeral())
            .expect("load workbook");

    assert_num(
        wb.evaluate_cell("Sheet1", 1, 2).expect("evaluate B1"),
        140.0,
    );
}

#[test]
fn umya_hidden_rows_subtotal_full_code_matrix_end_to_end() {
    let path = build_workbook(|book| {
        let sheet = book.get_sheet_by_name_mut("Sheet1").unwrap();
        sheet.get_cell_mut((1, 2)).set_value_number(10.0); // A2
        sheet.get_cell_mut((1, 3)).set_value_number(20.0); // A3 (hidden)
        sheet.get_cell_mut((1, 4)).set_value_number(30.0); // A4
        sheet.get_cell_mut((1, 5)).set_value_number(100.0); // A5
        sheet.get_row_dimension_mut(&3).set_hidden(true);

        let mut col = 2u32;
        for code in 1..=11 {
            sheet
                .get_cell_mut((col, 1))
                .set_formula(format!("=SUBTOTAL({code},A2:A5)"));
            col += 1;
        }
        for code in 101..=111 {
            sheet
                .get_cell_mut((col, 1))
                .set_formula(format!("=SUBTOTAL({code},A2:A5)"));
            col += 1;
        }
    });

    let backend = UmyaAdapter::open_path(&path).expect("open xlsx");
    let mut wb =
        Workbook::from_reader(backend, LoadStrategy::EagerAll, WorkbookConfig::ephemeral())
            .expect("load workbook");

    let include_all = [10.0, 20.0, 30.0, 100.0];
    let visible_only = [10.0, 30.0, 100.0];

    let mut col = 2u32;
    for code in 1..=11 {
        let expected = op_expected(code, &include_all);
        assert_num(
            wb.evaluate_cell("Sheet1", 1, col)
                .expect("evaluate SUBTOTAL include-all"),
            expected,
        );
        col += 1;
    }
    for code in 101..=111 {
        let expected = op_expected(code - 100, &visible_only);
        assert_num(
            wb.evaluate_cell("Sheet1", 1, col)
                .expect("evaluate SUBTOTAL hidden-aware"),
            expected,
        );
        col += 1;
    }
}

#[test]
fn umya_hidden_rows_aggregate_phase1_full_matrix_end_to_end() {
    let path = build_workbook(|book| {
        let sheet = book.get_sheet_by_name_mut("Sheet1").unwrap();
        sheet.get_cell_mut((1, 2)).set_value_number(10.0); // A2
        sheet.get_cell_mut((1, 3)).set_value_number(20.0); // A3 (hidden)
        sheet.get_cell_mut((1, 4)).set_value_number(30.0); // A4
        sheet.get_cell_mut((1, 5)).set_value_number(100.0); // A5
        sheet.get_row_dimension_mut(&3).set_hidden(true);

        let mut col = 2u32;
        for function_num in 1..=11 {
            for options in 0..=3 {
                sheet
                    .get_cell_mut((col, 1))
                    .set_formula(format!("=AGGREGATE({function_num},{options},A2:A5)"));
                col += 1;
            }
        }
    });

    let backend = UmyaAdapter::open_path(&path).expect("open xlsx");
    let mut wb =
        Workbook::from_reader(backend, LoadStrategy::EagerAll, WorkbookConfig::ephemeral())
            .expect("load workbook");

    let include_all = [10.0, 20.0, 30.0, 100.0];
    let visible_only = [10.0, 30.0, 100.0];

    let mut col = 2u32;
    for function_num in 1..=11 {
        for options in 0..=3 {
            let values = if options == 1 || options == 3 {
                &visible_only[..]
            } else {
                &include_all[..]
            };
            let expected = op_expected(function_num, values);
            assert_num(
                wb.evaluate_cell("Sheet1", 1, col)
                    .expect("evaluate AGGREGATE matrix"),
                expected,
            );
            col += 1;
        }
    }
}

fn evaluate_row_9(bytes: Vec<u8>) -> Vec<f64> {
    let adapter = UmyaAdapter::open_bytes(bytes).expect("open xlsx");
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
fn umya_autofilter_hidden_rows_are_skipped_by_every_subtotal() {
    let bytes = filtered_xlsx(
        r#"<sheetPr filterMode="1"/>"#,
        r#"<autoFilter ref="A1:A5"><filterColumn colId="0"><filters><filter val="10"/><filter val="40"/></filters></filterColumn></autoFilter>"#,
    );

    let mut adapter = UmyaAdapter::open_bytes(bytes.clone()).expect("open xlsx");
    let sheet = adapter.read_sheet("Data").expect("read sheet");
    assert_eq!(sheet.row_hidden_filter, vec![3, 4]);
    assert_eq!(sheet.row_hidden_manual, vec![7]);

    let ctx = formualizer_eval::test_workbook::TestWorkbook::new();
    let mut engine: Engine<_> = Engine::new(ctx, EvalConfig::default());
    adapter
        .stream_into_engine(&mut engine)
        .expect("stream into engine");
    assert_eq!(
        engine.is_row_hidden("Data", 3, Some(RowVisibilitySource::Filter)),
        Some(true)
    );
    assert_eq!(
        engine.is_row_hidden("Data", 3, Some(RowVisibilitySource::Manual)),
        Some(false)
    );
    assert_eq!(
        engine.is_row_hidden("Data", 7, Some(RowVisibilitySource::Manual)),
        Some(true)
    );

    // SUBTOTAL(9) keeps the manually hidden 100 but not the filtered 20 and
    // 30; SUBTOTAL(109) drops all three; SUM ignores visibility.
    assert_eq!(evaluate_row_9(bytes), vec![150.0, 50.0, 4.0, 200.0]);
}

/// The same packages and expectations as the calamine backend's
/// calamine_splits_saved_hidden_rows_by_filter, checked against that backend
/// directly when it is built.
#[test]
fn umya_splits_saved_hidden_rows_like_calamine() {
    for case in cases() {
        let mut adapter = UmyaAdapter::open_bytes(case.bytes.clone()).expect(case.name);
        let sheet = adapter.read_sheet("Data").expect(case.name);
        assert_eq!(sheet.row_hidden_filter, case.filter, "{}", case.name);
        assert_eq!(sheet.row_hidden_manual, case.manual, "{}", case.name);
        #[cfg(feature = "calamine")]
        {
            let mut calamine =
                formualizer_workbook::CalamineAdapter::open_bytes(case.bytes.clone())
                    .expect(case.name);
            let other = calamine.read_sheet("Data").expect(case.name);
            assert_eq!(
                other.row_hidden_filter, sheet.row_hidden_filter,
                "{}",
                case.name
            );
            assert_eq!(
                other.row_hidden_manual, sheet.row_hidden_manual,
                "{}",
                case.name
            );
        }
        assert_eq!(evaluate_row_9(case.bytes), case.row_9, "{}", case.name);
    }
}
