use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_PROJECT: AtomicU64 = AtomicU64::new(0);

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures")
        .join(name)
}

fn compile(name: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_omlua-driver"))
        .arg("emit-omir")
        .arg(fixture(name))
        .output()
        .expect("failed to run omlua-driver")
}

fn lua54_fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/lua54/fixtures")
        .join(name)
}

fn lua54_expected(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/lua54/expected")
        .join(name)
}

fn project_directory(name: &str) -> PathBuf {
    let sequence = NEXT_PROJECT.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "omlua-driver-test-{}-{sequence}-{name}",
        std::process::id()
    ));
    fs::create_dir(&path).expect("failed to create test project directory");
    path
}

fn build(project: &Path, source: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_omlua-driver"))
        .args(["build", "--backend", "lua54"])
        .arg(source)
        .current_dir(project)
        .output()
        .expect("failed to run omlua-driver")
}

fn artifact(project: &Path) -> PathBuf {
    project.join("target/omlua/program.lua")
}

#[test]
fn lowers_reachable_scalar_mir_to_exact_omir() {
    let output = compile("branch.rs");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());

    let stdout = String::from_utf8(output.stdout).expect("OMIR output is not UTF-8");
    assert_eq!(stdout, expected_omir());
}

#[test]
fn produces_identical_omir_on_repeated_compilation() {
    let first = compile("branch.rs");
    let second = compile("branch.rs");

    assert!(first.status.success());
    assert!(second.status.success());
    assert_eq!(first.stdout, second.stdout);
    assert_eq!(first.stderr, second.stderr);
}

#[test]
fn lowers_every_initial_scalar_operator() {
    let output = compile("operators.rs");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());

    let stdout = String::from_utf8(output.stdout).expect("OMIR output is not UTF-8");
    for operation in [
        "checked_add",
        "checked_sub",
        "checked_mul",
        "div ",
        "rem ",
        "eq ",
        "ne ",
        "lt ",
        "le ",
        "gt ",
        "ge ",
        "not ",
        "and ",
        "overflow_div",
        "overflow_rem",
    ] {
        assert!(
            stdout.contains(operation),
            "missing operation `{operation}`"
        );
    }

    assert!(stdout.contains(concat!(
        "  bb4:\n",
        "    %14 = eq copy %2, -1_i32\n",
        "    %15 = eq copy %9, -2147483648_i32\n",
        "    %16 = and move %14, move %15\n",
        "    assert move %16 == false overflow_div(copy %9, copy %2) -> bb5 unwind continue\n",
        "  bb5:\n",
        "    %12 = div copy %9, copy %2\n",
        "    %17 = eq copy %2, 0_i32\n",
        "    assert move %17 == false remainder_by_zero(copy %12) -> bb6 unwind continue\n",
    )));
    assert!(stdout.contains(concat!(
        "  bb6:\n",
        "    %18 = eq copy %2, -1_i32\n",
        "    %19 = eq copy %12, -2147483648_i32\n",
        "    %20 = and move %18, move %19\n",
        "    assert move %20 == false overflow_rem(copy %12, copy %2) -> bb7 unwind continue\n",
        "  bb7:\n",
        "    %0 = rem copy %12, copy %2\n",
        "    return\n",
    )));
}

#[test]
fn rejects_generic_calls_without_partial_output() {
    let output = compile("unsupported_generic.rs");
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());

    let stderr = String::from_utf8(output.stderr).expect("OMLua diagnostic is not UTF-8");
    assert!(stderr.contains("generic function `identity` is not supported"));
    assert!(stderr.contains("in function `main`, basic block bb0"));
}

#[test]
fn rejects_associated_calls_without_partial_output() {
    let output = compile("unsupported_associated.rs");
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());

    let stderr = String::from_utf8(output.stderr).expect("OMLua diagnostic is not UTF-8");
    assert!(stderr.contains("trait method `Operations::add` is not supported"));
    assert!(stderr.contains("in function `main`, basic block bb0"));
}

#[test]
fn lowers_named_structs_shared_references_and_inherent_methods() {
    let output = compile("struct_method.rs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());

    let omir = String::from_utf8(output.stdout).unwrap();
    assert_eq!(omir, expected_struct_omir());
}

#[test]
fn rejects_mutable_references_and_tuple_structs_without_partial_output() {
    let mutable = compile("unsupported_mut_reference.rs");
    assert!(!mutable.status.success());
    assert!(mutable.stdout.is_empty());
    assert!(
        String::from_utf8(mutable.stderr)
            .unwrap()
            .contains("mutable reference `&mut Counter` is not supported")
    );

    let tuple = compile("unsupported_tuple_struct.rs");
    assert!(!tuple.status.success());
    assert!(tuple.stdout.is_empty());
    assert!(
        String::from_utf8(tuple.stderr)
            .unwrap()
            .contains("tuple struct `Pair` is not supported")
    );
}

#[test]
fn lowers_nested_struct_field_paths() {
    let output = compile("nested_struct.rs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let omir = String::from_utf8(output.stdout).unwrap();
    assert!(omir.contains("struct @0 Point"));
    assert!(omir.contains("struct @1 Holder"));
    assert!(omir.contains("borrow_shared (*%1).0"));
    assert!(omir.contains("copy (*%1).0"));
    assert!(omir.contains("copy (*%1).1"));
}

#[test]
fn lowers_a_whole_copy_struct_dereference() {
    let output = compile("deref_copy_struct.rs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let omir = String::from_utf8(output.stdout).unwrap();
    assert!(omir.contains("copy *%1"));
}

#[test]
fn builds_and_executes_a_method_borrowed_from_a_nested_field() {
    let project = project_directory("nested-struct-method");
    let output = build(&project, &fixture("nested_struct.rs"));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let execution = Command::new("lua")
        .arg(artifact(&project))
        .output()
        .expect("Lua 5.4.8 is required on PATH");
    assert!(
        execution.status.success(),
        "{}",
        String::from_utf8_lossy(&execution.stderr)
    );
    assert!(execution.stdout.is_empty());
    assert!(execution.stderr.is_empty());
    fs::remove_dir_all(project).unwrap();
}

#[test]
fn rejects_generic_structs_and_reference_fields_without_partial_output() {
    let generic = compile("unsupported_generic_struct.rs");
    assert!(!generic.status.success());
    assert!(generic.stdout.is_empty());
    assert!(
        String::from_utf8(generic.stderr)
            .unwrap()
            .contains("generic structure `Wrapper` is not supported")
    );

    let reference_field = compile("unsupported_reference_field.rs");
    assert!(!reference_field.status.success());
    assert!(reference_field.stdout.is_empty());
    assert!(
        String::from_utf8(reference_field.stderr)
            .unwrap()
            .contains("reference field `value` in structure `Holder` is not supported")
    );
}

#[test]
fn rejects_i64_without_partial_output() {
    let output = compile("unsupported_i64.rs");
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());

    let stderr = String::from_utf8(output.stderr).expect("OMLua diagnostic is not UTF-8");
    assert_eq!(
        stderr,
        concat!(
            "error[OMLUA0001]: local _1: type `i64` is not supported\n",
            "  in function `main`\n",
        )
    );
}

#[test]
fn rejects_references_without_partial_output() {
    let output = compile("unsupported_reference.rs");
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());

    let stderr = String::from_utf8(output.stderr).expect("OMLua diagnostic is not UTF-8");
    assert_eq!(
        stderr,
        concat!(
            "error[OMLUA0001]: local _3: shared reference `&i32` is not supported; only references to named structures are supported\n",
            "  in function `main`\n",
        )
    );
}

#[test]
fn rejects_external_calls_without_partial_output() {
    let output = compile("unsupported_external.rs");
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());

    let stderr = String::from_utf8(output.stderr).expect("OMLua diagnostic is not UTF-8");
    assert!(stderr.contains("external call `std::hint::black_box` is not supported"));
    assert!(stderr.contains("in function `main`, basic block bb0"));
}

#[test]
fn preserves_rustc_failure_status_and_diagnostic() {
    let output = compile("type_error.rs");
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());

    let stderr = String::from_utf8(output.stderr).expect("rustc diagnostic is not UTF-8");
    assert!(stderr.contains("error[E0308]: mismatched types"));
    assert!(stderr.contains("expected `i32`, found `&str`"));
}

#[test]
fn builds_the_exact_reviewed_lua54_artifact() {
    let project = project_directory("exact-output");
    let output = build(&project, &lua54_fixture("scalars.rs"));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    assert_eq!(
        fs::read(artifact(&project)).unwrap(),
        fs::read(lua54_expected("scalars.lua")).unwrap()
    );
    let mut output_entries: Vec<_> = fs::read_dir(project.join("target/omlua"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    output_entries.sort();
    assert_eq!(output_entries, ["program.lua"]);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(PathBuf::from(stdout.trim()), artifact(&project));
    fs::remove_dir_all(project).unwrap();
}

#[test]
fn failed_build_removes_stale_artifact_without_touching_siblings() {
    let project = project_directory("stale-output");
    let output_directory = project.join("target/omlua");
    fs::create_dir_all(&output_directory).unwrap();
    fs::write(artifact(&project), "stale Lua").unwrap();
    fs::write(output_directory.join("keep.txt"), "keep").unwrap();

    let output = build(&project, &fixture("type_error.rs"));
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(!artifact(&project).exists());
    assert_eq!(
        fs::read_to_string(output_directory.join("keep.txt")).unwrap(),
        "keep"
    );
    fs::remove_dir_all(project).unwrap();
}

#[test]
fn output_directory_failure_writes_no_partial_artifact() {
    let project = project_directory("output-failure");
    fs::create_dir(project.join("target")).unwrap();
    fs::write(project.join("target/omlua"), "not a directory").unwrap();

    let output = build(&project, &lua54_fixture("scalars.rs"));
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(!artifact(&project).exists());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("Lua output directory"));
    fs::remove_dir_all(project).unwrap();
}

#[test]
fn driver_produced_lua_executes_with_the_documented_interpreter() {
    let version = Command::new("lua")
        .arg("-v")
        .output()
        .expect("Lua 5.4.8 is required on PATH");
    assert!(version.status.success());
    let version_text = format!(
        "{}{}",
        String::from_utf8_lossy(&version.stdout),
        String::from_utf8_lossy(&version.stderr)
    );
    assert!(version_text.starts_with("Lua 5.4.8 "), "{version_text}");

    let project = project_directory("execute");
    let build_output = build(&project, &lua54_fixture("scalars.rs"));
    assert!(build_output.status.success());
    let execution = Command::new("lua")
        .arg(artifact(&project))
        .output()
        .expect("failed to execute generated Lua");
    assert!(
        execution.status.success(),
        "{}",
        String::from_utf8_lossy(&execution.stderr)
    );
    assert!(execution.stdout.is_empty());
    assert!(execution.stderr.is_empty());
    fs::remove_dir_all(project).unwrap();
}

#[test]
fn lowers_enums_variants_and_match_to_omir() {
    let output = compile("enum_match.rs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());

    let omir = String::from_utf8(output.stdout).unwrap();
    assert!(omir.contains("enum @0 Command {"));
    assert!(omir.contains("  v0 Stop\n"));
    assert!(omir.contains("  v1 Pause\n"));
    assert!(omir.contains("  v2 GoTo {\n"));
    assert!(omir.contains("  v3 SetThrottle {\n"));
    assert!(omir.contains("variant @0#2 { 20_i32, 22_i32 }"));
    assert!(omir.contains("discriminant "));
    assert!(omir.contains("copy %1#2.0"));
    assert!(omir.contains("copy %1#2.1"));
    assert!(omir.contains("copy %1#3.0"));
}

#[test]
fn lowers_monomorphized_std_option_and_result_enums() {
    let output = compile("option_result.rs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());

    let omir = String::from_utf8(output.stdout).unwrap();
    assert!(omir.contains("enum @0 Option<i32> {"));
    assert!(omir.contains("  v0 None\n"));
    assert!(omir.contains("  v1 Some {\n    .0 0: i32\n  }\n"));
    assert!(omir.contains("enum @1 Option<Result<i32, MyErr>> {"));
    assert!(omir.contains("    .0 0: enum @2\n"));
    assert!(omir.contains("enum @2 Result<i32, MyErr> {"));
    assert!(omir.contains("  v0 Ok {\n    .0 0: i32\n  }\n"));
    assert!(omir.contains("  v1 Err {\n    .0 0: struct @3\n  }\n"));
    assert!(omir.contains("variant @0#1 { 21_i32 }"));
    assert!(omir.contains("variant @0#0 {  }"));
    assert!(omir.contains("variant @2#0 { 7_i32 }"));
    assert!(omir.contains("variant @1#1 { move %10 }"));
    assert!(omir.contains("copy %9#1.0#0.0"));
    assert!(omir.contains("%0 = variant @2#1 { move %6 }"));
    assert!(omir.contains("%0 = variant @2#0 { move %3 }"));
}

#[test]
fn lowers_question_mark_operator_to_synthetic_try_helpers() {
    let output = compile("question_mark.rs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());

    let omir = String::from_utf8(output.stdout).unwrap();
    assert!(omir.contains("enum @0 Result<i32, MyErr> {"));
    assert!(omir.contains("enum @2 Option<i32> {"));
    assert!(omir.contains("enum @3 ControlFlow<Result<Infallible, MyErr>, i32> {"));
    assert!(omir.contains("enum @4 Result<Infallible, MyErr> {"));
    assert!(omir.contains("    .0 0: enum @5\n"));
    assert!(omir.contains("enum @5 Infallible {\n}"));
    assert!(omir.contains("enum @6 ControlFlow<Option<Infallible>, i32> {"));
    assert!(omir.contains("enum @7 Option<Infallible> {"));
    assert!(omir.contains("fn @4 __omlua_result_branch<i32, MyErr>(%1) -> enum @3 {"));
    assert!(omir.contains("fn @5 __omlua_result_from_residual<i32, MyErr>(%1) -> enum @0 {"));
    assert!(omir.contains("fn @7 __omlua_option_branch<i32>(%1) -> enum @6 {"));
    assert!(omir.contains("fn @8 __omlua_option_from_residual<i32>() -> enum @2 {"));
    assert!(omir.contains("discriminant copy %1"));
    assert!(omir.contains("switch move %2 [0: bb2, 1: bb1, otherwise: bb3]"));
    assert!(omir.contains("switch move %2 [0: bb1, 1: bb2, otherwise: bb3]"));
    assert!(omir.contains("%3 = variant @6#0 { copy %1#1.0 }"));
    assert!(omir.contains("%4 = variant @7#0 {  }"));
    assert!(omir.contains("%5 = variant @6#1 { move %4 }"));
    assert!(omir.contains("%3 = variant @3#0 { copy %1#0.0 }"));
    assert!(omir.contains("%4 = variant @4#1 { copy %1#1.0 }"));
    assert!(omir.contains("%5 = variant @3#1 { move %4 }"));
    assert!(omir.contains("%0 = variant @2#0 {  }"));
    assert!(omir.contains("%0 = variant @0#1 { move %1#1.0 }"));
    assert!(omir.contains("call @4(move %2)"));
    assert!(omir.contains("call @4(move %7)"));
    assert!(omir.contains("call @5(copy %4)"));
    assert!(omir.contains("call @7(move %2)"));
    assert!(omir.contains("call @8()"));
}

#[test]
fn lowers_while_loop_break_continue_and_value_loop() {
    let output = compile("while_loop.rs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());

    let omir = String::from_utf8(output.stdout).unwrap();
    assert!(omir.contains("switch move %3 [0: bb7, otherwise: bb2]"));
    assert!(omir.contains("switch move %7 [0: bb4, otherwise: bb1]"));
    assert!(omir.contains("switch move %9 [0: bb5, otherwise: bb7]"));
    assert!(omir.contains("switch move %15 [0: bb9, otherwise: bb8]"));
    assert!(omir.contains("switch move %3"));
    assert!(omir.contains("  bb6:\n    %2 = move %12\n    goto bb1"));
    assert!(omir.contains("  bb10:\n    %2 = move %17\n    goto bb7"));
    assert!(omir.contains("%14 = copy %2"));
    assert!(omir.contains("(%5, %6) = checked_add copy %1, 1_i32"));
    assert!(omir.contains("(%12, %13) = checked_add copy %2, copy %11"));
    assert!(omir.contains("(%22, %23) = checked_sub copy %21, 8_i32"));
    assert!(omir.contains("assert move %23 == false overflow_sub(move %21, 8_i32) -> bb12 unwind continue"));
    assert!(omir.contains("assert move %18 == false overflow_add(copy %2, 1_i32) -> bb10 unwind continue"));
}

#[test]
fn builds_and_executes_the_exact_while_loop_artifact() {
    let project = project_directory("while-loop");
    let output = build(&project, &lua54_fixture("while_loop.rs"));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    assert_eq!(
        fs::read(artifact(&project)).unwrap(),
        fs::read(lua54_expected("while_loop.lua")).unwrap()
    );

    let execution = Command::new("lua")
        .arg(artifact(&project))
        .output()
        .expect("Lua 5.4.8 is required on PATH");
    assert!(
        execution.status.success(),
        "{}",
        String::from_utf8_lossy(&execution.stderr)
    );
    assert!(execution.stdout.is_empty());
    assert!(execution.stderr.is_empty());
    fs::remove_dir_all(project).unwrap();
}

#[test]
fn lowers_for_loops_over_integer_ranges() {
    let output = compile("for_range.rs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());

    let omir = String::from_utf8(output.stdout).unwrap();
    assert!(omir.contains("struct @0 Range<i32> {"));
    assert!(omir.contains("  .0 start: i32\n"));
    assert!(omir.contains("  .1 end: i32\n"));
    assert!(omir.contains("enum @1 Option<i32> {"));
    assert!(omir.contains("%3 = struct @0 { 0_i32, 10_i32 }"));
    assert!(omir.contains("%2 = call @1(move %3)"));
    assert!(omir.contains("%16 = lt copy %14, copy %15"));
    assert!(omir.contains("switch move %16 [0: bb9, 1: bb10, otherwise: bb11]"));
    assert!(omir.contains("%17 = add copy %14, 1_i32"));
    assert!(omir.contains("%4 = struct @0 { copy %17, copy %15 }"));
    assert!(omir.contains("%5 = variant @1#1 { move %17 }"));
    assert!(omir.contains("  bb9:\n    goto bb3"));
    assert!(omir.contains("  bb10:\n    %17 = add copy %14, 1_i32"));
    assert!(omir.contains("  bb7:\n    %1 = move %9\n    goto bb2"));
    assert!(omir.contains("fn @1 __omlua_range_into_iter<i32>(%1) -> struct @0 {"));
    assert!(omir.contains("  bb0:\n    %0 = move %1\n    return"));
}

#[test]
fn rejects_external_enums_outside_the_try_whitelist_without_partial_output() {
    let external = compile("unsupported_external_enum.rs");
    assert!(!external.status.success());
    assert!(external.stdout.is_empty());
    assert!(
        String::from_utf8(external.stderr)
            .unwrap()
            .contains("external enum `std::cmp::Ordering` is not supported")
    );
}

#[test]
fn rejects_enum_references_and_ref_mut_bindings_without_partial_output() {
    let enum_ref = compile("unsupported_enum_ref.rs");
    assert!(!enum_ref.status.success());
    assert!(enum_ref.stdout.is_empty());
    assert!(
        String::from_utf8(enum_ref.stderr)
            .unwrap()
            .contains(
                "shared reference `&Command` is not supported; only references to named structures are supported"
            )
    );

    let ref_mut = compile("unsupported_ref_mut.rs");
    assert!(!ref_mut.status.success());
    assert!(ref_mut.stdout.is_empty());
    assert!(
        String::from_utf8(ref_mut.stderr)
            .unwrap()
            .contains("mutable reference `&mut i32` is not supported")
    );

    let generic = compile("unsupported_generic_enum.rs");
    assert!(!generic.status.success());
    assert!(generic.stdout.is_empty());
    assert!(
        String::from_utf8(generic.stderr)
            .unwrap()
            .contains("generic enum `Wrapper` is not supported")
    );
}

fn expected_omir() -> &'static str {
    concat!(
        "program entry @0\n",
        "\n",
        "fn @0 main() -> unit {\n",
        "  locals:\n",
        "    %0: unit return\n",
        "    %1: i32 temporary\n",
        "    %2: i32 temporary\n",
        "  bb0:\n",
        "    %1 = call @1(-5_i32, 2_i32) -> bb1 unwind continue\n",
        "  bb1:\n",
        "    %2 = call @2(copy %1) -> bb2 unwind continue\n",
        "  bb2:\n",
        "    return\n",
        "}\n",
        "\n",
        "fn @1 add(%1, %2) -> i32 {\n",
        "  locals:\n",
        "    %0: i32 return\n",
        "    %1: i32 parameter\n",
        "    %2: i32 parameter\n",
        "    %3: i32 checked-value\n",
        "    %4: bool checked-overflow\n",
        "  bb0:\n",
        "    (%3, %4) = checked_add copy %1, copy %2\n",
        "    assert move %4 == false overflow_add(copy %1, copy %2) -> bb1 unwind continue\n",
        "  bb1:\n",
        "    %0 = move %3\n",
        "    return\n",
        "}\n",
        "\n",
        "fn @2 absolute(%1) -> i32 {\n",
        "  locals:\n",
        "    %0: i32 return\n",
        "    %1: i32 parameter\n",
        "    %2: bool temporary\n",
        "    %3: bool temporary\n",
        "  bb0:\n",
        "    %2 = ge copy %1, 0_i32\n",
        "    switch move %2 [0: bb2, otherwise: bb1]\n",
        "  bb1:\n",
        "    %0 = copy %1\n",
        "    goto bb4\n",
        "  bb2:\n",
        "    %3 = eq copy %1, -2147483648_i32\n",
        "    assert move %3 == false overflow_neg(copy %1) -> bb3 unwind continue\n",
        "  bb3:\n",
        "    %0 = neg copy %1\n",
        "    goto bb4\n",
        "  bb4:\n",
        "    return\n",
        "}\n",
    )
}

#[test]
fn builds_and_executes_the_exact_struct_method_artifact() {
    let project = project_directory("struct-method");
    let output = build(&project, &lua54_fixture("struct_method.rs"));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    assert_eq!(
        fs::read(artifact(&project)).unwrap(),
        fs::read(lua54_expected("struct_method.lua")).unwrap()
    );

    let execution = Command::new("lua")
        .arg(artifact(&project))
        .output()
        .expect("Lua 5.4.8 is required on PATH");
    assert!(
        execution.status.success(),
        "{}",
        String::from_utf8_lossy(&execution.stderr)
    );
    assert!(execution.stdout.is_empty());
    assert!(execution.stderr.is_empty());
    fs::remove_dir_all(project).unwrap();
}

#[test]
fn builds_and_executes_the_exact_enum_match_artifact() {
    let project = project_directory("enum-match");
    let output = build(&project, &lua54_fixture("enum_match.rs"));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    assert_eq!(
        fs::read(artifact(&project)).unwrap(),
        fs::read(lua54_expected("enum_match.lua")).unwrap()
    );

    let execution = Command::new("lua")
        .arg(artifact(&project))
        .output()
        .expect("Lua 5.4.8 is required on PATH");
    assert!(
        execution.status.success(),
        "{}",
        String::from_utf8_lossy(&execution.stderr)
    );
    assert!(execution.stdout.is_empty());
    assert!(execution.stderr.is_empty());
    fs::remove_dir_all(project).unwrap();
}

#[test]
fn builds_and_executes_the_exact_option_result_artifact() {
    let project = project_directory("option-result");
    let output = build(&project, &lua54_fixture("option_result.rs"));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    assert_eq!(
        fs::read(artifact(&project)).unwrap(),
        fs::read(lua54_expected("option_result.lua")).unwrap()
    );

    let execution = Command::new("lua")
        .arg(artifact(&project))
        .output()
        .expect("Lua 5.4.8 is required on PATH");
    assert!(
        execution.status.success(),
        "{}",
        String::from_utf8_lossy(&execution.stderr)
    );
    assert!(execution.stdout.is_empty());
    assert!(execution.stderr.is_empty());
    fs::remove_dir_all(project).unwrap();
}

#[test]
fn builds_and_executes_the_exact_question_mark_artifact() {
    let project = project_directory("question-mark");
    let output = build(&project, &lua54_fixture("question_mark.rs"));
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    assert_eq!(
        fs::read(artifact(&project)).unwrap(),
        fs::read(lua54_expected("question_mark.lua")).unwrap()
    );

    let execution = Command::new("lua")
        .arg(artifact(&project))
        .output()
        .expect("Lua 5.4.8 is required on PATH");
    assert!(
        execution.status.success(),
        "{}",
        String::from_utf8_lossy(&execution.stderr)
    );
    assert!(execution.stdout.is_empty());
    assert!(execution.stderr.is_empty());
    fs::remove_dir_all(project).unwrap();
}

fn expected_struct_omir() -> &'static str {
    concat!(
        "program entry @0\n",
        "\n",
        "struct @0 Point {\n",
        "  .0 x: i32\n",
        "  .1 y: i32\n",
        "}\n",
        "\n",
        "fn @0 main() -> unit {\n",
        "  locals:\n",
        "    %0: unit return\n",
        "    %1: struct @0 temporary\n",
        "    %2: i32 temporary\n",
        "    %3: &struct @0 temporary\n",
        "  bb0:\n",
        "    %1 = struct @0 { 20_i32, 22_i32 }\n",
        "    %3 = borrow_shared %1\n",
        "    %2 = call @1(copy %3) -> bb1 unwind continue\n",
        "  bb1:\n",
        "    return\n",
        "}\n",
        "\n",
        "fn @1 verify(%1) -> i32 {\n",
        "  locals:\n",
        "    %0: i32 return\n",
        "    %1: &struct @0 parameter\n",
        "  bb0:\n",
        "    %0 = call @2(copy %1) -> bb1 unwind continue\n",
        "  bb1:\n",
        "    return\n",
        "}\n",
        "\n",
        "fn @2 Point::sum(%1) -> i32 {\n",
        "  locals:\n",
        "    %0: i32 return\n",
        "    %1: &struct @0 parameter\n",
        "    %2: i32 temporary\n",
        "    %3: i32 temporary\n",
        "    %4: i32 checked-value\n",
        "    %5: bool checked-overflow\n",
        "  bb0:\n",
        "    %2 = copy (*%1).0\n",
        "    %3 = copy (*%1).1\n",
        "    (%4, %5) = checked_add copy %2, copy %3\n",
        "    assert move %5 == false overflow_add(move %2, move %3) -> bb1 unwind continue\n",
        "  bb1:\n",
        "    %0 = move %4\n",
        "    return\n",
        "}\n",
    )
}
