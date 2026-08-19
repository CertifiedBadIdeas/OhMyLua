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
