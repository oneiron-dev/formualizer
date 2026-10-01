use formualizer_workbook::{LiteralValue, Workbook, WorkbookConfig};

#[test]
fn whole_axis_blank_counts_include_spill_members_beyond_graph_bounds() {
    use formualizer_eval::engine::FormulaPlaneMode;
    for mode in [
        FormulaPlaneMode::Off,
        FormulaPlaneMode::Shadow,
        FormulaPlaneMode::AuthoritativeExperimental,
    ] {
        let mut config = WorkbookConfig::interactive();
        config.eval.formula_plane_mode = mode;
        let mut wb = Workbook::new_with_config(config);
        wb.add_sheet("Data").unwrap();
        wb.add_sheet("Results").unwrap();
        wb.set_formula("Data", 10, 3, "SEQUENCE(2,3)").unwrap();
        let formulas = [
            r#"COUNTIF(Data!A1:F20,"")"#,
            "COUNTBLANK(Data!A1:F20)",
            r#"COUNTIF(Data!C:C,"")"#,
            "COUNTBLANK(Data!10:10)",
            r#"COUNTIF(Data!E:E,"<>")"#,
            "COUNTBLANK(Data!11:11)",
            r#"COUNTIF(Data!C:C,">0")"#,
        ];
        for (index, formula) in formulas.iter().enumerate() {
            wb.set_formula("Results", index as u32 + 1, 1, formula)
                .unwrap();
        }
        wb.evaluate_all().unwrap();
        assert_eq!(wb.get_value("Data", 11, 5), Some(LiteralValue::Number(6.0)));
        for (index, expected) in [114.0, 114.0, 1_048_574.0, 16_381.0, 2.0, 16_381.0, 2.0]
            .into_iter()
            .enumerate()
        {
            assert_eq!(
                wb.get_value("Results", index as u32 + 1, 1),
                Some(LiteralValue::Number(expected)),
                "{mode:?}: {}",
                formulas[index]
            );
        }
        wb.set_formula("Data", 10, 3, "9").unwrap();
        wb.evaluate_all().unwrap();
        for (index, expected) in [119.0, 119.0, 1_048_575.0, 16_383.0, 0.0, 16_384.0, 1.0]
            .into_iter()
            .enumerate()
        {
            assert_eq!(
                wb.get_value("Results", index as u32 + 1, 1),
                Some(LiteralValue::Number(expected)),
                "after clear {mode:?}: {}",
                formulas[index]
            );
        }
    }
}

#[test]
fn blank_counts_keep_wide_logical_extents_arithmetic() {
    let mut wb = Workbook::new_with_config(WorkbookConfig::ephemeral());
    wb.add_sheet("Data").unwrap();
    wb.add_sheet("Results").unwrap();
    wb.set_value("Data", 5, 3, LiteralValue::Number(1.0))
        .unwrap();
    let cases = [
        (r#"COUNTBLANK(Data!A:XFD)"#, 17_179_869_183.0),
        (r#"COUNTBLANK(Data!1:1048576)"#, 17_179_869_183.0),
        (r#"COUNTIF(Data!A:XFD,"")"#, 17_179_869_183.0),
        (r#"COUNTIF(Data!1:1048576,"")"#, 17_179_869_183.0),
        (r#"COUNTBLANK(Data!C:C)"#, 1_048_575.0),
        (r#"COUNTIF(Data!C:C,"")"#, 1_048_575.0),
        (r#"COUNTBLANK(Data!5:5)"#, 16_383.0),
        (r#"COUNTIF(Data!5:5,"")"#, 16_383.0),
    ];
    for (row, (formula, _)) in cases.iter().enumerate() {
        wb.set_formula("Results", row as u32 + 1, 1, formula)
            .unwrap();
    }
    wb.evaluate_all().unwrap();
    for (row, (formula, expected)) in cases.iter().enumerate() {
        assert_eq!(
            wb.get_value("Results", row as u32 + 1, 1),
            Some(LiteralValue::Number(*expected)),
            "{formula}"
        );
    }
}

#[test]
fn numeric_not_equal_counts_stored_and_unstored_blanks() {
    // A blank is not equal to a number, so COUNTIF(range,"<>n") counts the
    // blanks inside the stored rows as well as the unstored tail.
    let mut wb = Workbook::new_with_config(WorkbookConfig::ephemeral());
    wb.add_sheet("Data").unwrap();
    wb.add_sheet("Results").unwrap();
    wb.set_value("Data", 3, 1, LiteralValue::Number(-29.0))
        .unwrap();
    wb.set_value("Data", 4, 1, LiteralValue::Number(0.0))
        .unwrap();
    let cases = [
        (r#"COUNTIF(Data!A:A,"<>0")"#, 1_048_575.0),
        (r#"COUNTIF(Data!A:A,"<>-29")"#, 1_048_575.0),
        (r#"COUNTIF(Data!A1:A4,"<>0")"#, 3.0),
        (r#"COUNTIF(Data!A1:A10,"<>0")"#, 9.0),
        (r#"COUNTIF(Data!A2,"<>0")"#, 1.0),
        (r#"COUNTIF(Data!A:A,"0")"#, 1.0),
    ];
    for (row, (formula, _)) in cases.iter().enumerate() {
        wb.set_formula("Results", row as u32 + 1, 1, formula)
            .unwrap();
    }
    wb.evaluate_all().unwrap();
    for (row, (formula, expected)) in cases.iter().enumerate() {
        assert_eq!(
            wb.get_value("Results", row as u32 + 1, 1),
            Some(LiteralValue::Number(*expected)),
            "{formula}"
        );
    }
}

#[test]
fn countifs_counts_unstored_blank_rows_and_logicals_are_not_numbers() {
    // COUNTIFS counts every cell of its criteria ranges that meets all
    // criteria, the blanks past the last stored row included. Criteria
    // compare like types: FALSE is not 0, so "<>0" counts it.
    let mut wb = Workbook::new_with_config(WorkbookConfig::ephemeral());
    wb.add_sheet("Data").unwrap();
    wb.add_sheet("Results").unwrap();
    wb.set_value("Data", 3, 1, LiteralValue::Number(-29.0))
        .unwrap();
    wb.set_value("Data", 4, 1, LiteralValue::Number(0.0))
        .unwrap();
    wb.set_value("Data", 5, 1, LiteralValue::Boolean(false))
        .unwrap();
    wb.set_value("Data", 6, 1, LiteralValue::Boolean(true))
        .unwrap();
    let cases = [
        (r#"COUNTIFS(Data!A1:A10,"<>0")"#, 9.0),
        (r#"COUNTIFS(Data!A:A,"<>0")"#, 1_048_575.0),
        (r#"COUNTIFS(Data!A1:A10,"<>0",Data!A1:A10,"<>-29")"#, 8.0),
        (r#"COUNTIFS(Data!A1:A10,"")"#, 6.0),
        (r#"COUNTIFS(Data!A:A,"<>1")"#, 1_048_576.0),
        (r#"COUNTIF(Data!A1:A5,"<>0")"#, 4.0),
        (r#"COUNTIF(Data!A1:A6,"<>1")"#, 6.0),
        (r#"COUNTIF(Data!A:A,0)"#, 1.0),
        (r#"COUNTIF(Data!A:A,1)"#, 0.0),
        (r#"COUNTIF(Data!A:A,TRUE)"#, 1.0),
        (r#"COUNTIFS(Data!A:A,"<1")"#, 2.0),
        (r#"COUNTIF(Data!A:A,">=0")"#, 1.0),
    ];
    for (row, (formula, _)) in cases.iter().enumerate() {
        wb.set_formula("Results", row as u32 + 1, 1, formula)
            .unwrap();
    }
    wb.evaluate_all().unwrap();
    for (row, (formula, expected)) in cases.iter().enumerate() {
        assert_eq!(
            wb.get_value("Results", row as u32 + 1, 1),
            Some(LiteralValue::Number(*expected)),
            "{formula}"
        );
    }
}
