use super::common::arrow_eval_config;
use crate::engine::Engine;
use crate::test_workbook::TestWorkbook;
use crate::traits::EvaluationContext;
use arrow_array::Array;
use formualizer_common::{ExcelError, LiteralValue};
use formualizer_parse::parser::ReferenceType;

#[test]
fn blank_masks_use_cell_types_across_base_overlay_and_chunk_slices() {
    let values = vec![
        LiteralValue::Number(1.0),
        LiteralValue::Boolean(true),
        LiteralValue::Empty,
        LiteralValue::Text(String::new()),
        LiteralValue::Text("x".into()),
        LiteralValue::Error(ExcelError::new_na()),
        LiteralValue::Number(0.0),
        LiteralValue::Boolean(false),
    ];
    for overlay in [false, true] {
        let mut engine = Engine::new(TestWorkbook::new(), arrow_eval_config());
        {
            let mut ingest = engine.begin_bulk_ingest_arrow();
            ingest.add_sheet("S", 1, 3);
            for v in &values {
                ingest
                    .append_row(
                        "S",
                        &[if overlay {
                            LiteralValue::Text("old".into())
                        } else {
                            v.clone()
                        }],
                    )
                    .unwrap();
            }
            ingest.finish().unwrap();
        }
        if overlay {
            for (i, v) in values.iter().enumerate() {
                engine
                    .set_cell_value("S", i as u32 + 1, 1, v.clone())
                    .unwrap();
            }
        }
        for (start, end) in [(1, 8), (2, 7), (3, 4), (1, 2)] {
            let range =
                ReferenceType::range(Some("S".into()), Some(start), Some(1), Some(end), Some(1));
            let view = engine.resolve_range_view(&range, "S").unwrap();
            for criterion in ["", "<>"] {
                let pred =
                    crate::args::parse_criteria(&LiteralValue::Text(criterion.into())).unwrap();
                for _ in 0..3 {
                    let mask = engine.build_criteria_mask(&view, 0, &pred).unwrap();
                    assert_eq!(mask.len(), (end - start + 1) as usize);
                    for (i, value) in values[(start - 1) as usize..end as usize]
                        .iter()
                        .enumerate()
                    {
                        let blank = matches!(value, LiteralValue::Empty)
                            || matches!(value, LiteralValue::Text(s) if s.is_empty());
                        assert_eq!(
                            mask.is_valid(i) && mask.value(i),
                            if criterion.is_empty() { blank } else { !blank },
                            "overlay={overlay} range={start}:{end} criterion={criterion} row={i}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn numeric_ne_masks_count_blanks_across_base_overlay_and_chunk_slices() {
    // No text in the column, so the numeric criteria take the numeric Arrow
    // lane. Expected values are Excel's, not the scalar matcher's: criteria
    // compare like types, so a blank, a logical or an error is never equal to
    // a number. `<>n` matches all of them (TRUE for `<>1`, FALSE for `<>0`),
    // while `=n` and `>n` match numbers only.
    let values = vec![
        LiteralValue::Empty,
        LiteralValue::Number(0.0),
        LiteralValue::Empty,
        LiteralValue::Number(-29.0),
        LiteralValue::Boolean(true),
        LiteralValue::Error(ExcelError::new_na()),
        LiteralValue::Int(5),
        LiteralValue::Empty,
        LiteralValue::Boolean(false),
        LiteralValue::Number(1.0),
    ];
    type Excel = fn(Option<f64>) -> bool;
    let criteria: [(&str, Excel); 9] = [
        ("<>0", |n| n != Some(0.0)),
        ("<>1", |n| n != Some(1.0)),
        ("<>-29", |n| n != Some(-29.0)),
        ("<>5", |n| n != Some(5.0)),
        ("=0", |n| n == Some(0.0)),
        ("=1", |n| n == Some(1.0)),
        (">-30", |n| n.is_some_and(|n| n > -30.0)),
        (">=0", |n| n.is_some_and(|n| n >= 0.0)),
        ("<1", |n| n.is_some_and(|n| n < 1.0)),
    ];
    for overlay in [false, true] {
        let mut engine = Engine::new(TestWorkbook::new(), arrow_eval_config());
        {
            let mut ingest = engine.begin_bulk_ingest_arrow();
            ingest.add_sheet("S", 1, 3);
            for v in &values {
                ingest
                    .append_row(
                        "S",
                        &[if overlay {
                            LiteralValue::Number(7.0)
                        } else {
                            v.clone()
                        }],
                    )
                    .unwrap();
            }
            ingest.finish().unwrap();
        }
        if overlay {
            for (i, v) in values.iter().enumerate() {
                engine
                    .set_cell_value("S", i as u32 + 1, 1, v.clone())
                    .unwrap();
            }
        }
        for (start, end) in [(1, 10), (2, 7), (3, 3), (1, 2), (5, 9)] {
            let range =
                ReferenceType::range(Some("S".into()), Some(start), Some(1), Some(end), Some(1));
            let view = engine.resolve_range_view(&range, "S").unwrap();
            for (criterion, excel) in criteria {
                let pred =
                    crate::args::parse_criteria(&LiteralValue::Text(criterion.into())).unwrap();
                let mask = engine.build_criteria_mask(&view, 0, &pred).unwrap();
                assert_eq!(mask.len(), (end - start + 1) as usize);
                for (i, value) in values[(start - 1) as usize..end as usize]
                    .iter()
                    .enumerate()
                {
                    let number = match value {
                        LiteralValue::Number(n) => Some(*n),
                        LiteralValue::Int(n) => Some(*n as f64),
                        _ => None,
                    };
                    let expected = excel(number);
                    assert_eq!(
                        mask.is_valid(i) && mask.value(i),
                        expected,
                        "mask: overlay={overlay} range={start}:{end} criterion={criterion} row={i} value={value:?}"
                    );
                    // The scalar matcher (text columns, MAXIFS/MINIFS, the
                    // D-functions) gives the same answer.
                    assert_eq!(
                        crate::builtins::criteria_match(&pred, value),
                        expected,
                        "scalar: criterion={criterion} value={value:?}"
                    );
                }
            }
        }
    }
}
