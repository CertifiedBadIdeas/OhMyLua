use omlua_ir::{
    AssertKind, BinaryOp, BlockId, CheckedBinaryOp, Constant, FieldId, FunctionId, LocalId,
    LocalKind, OmBlock, OmField, OmFunction, OmLocal, OmProgram, OmStruct, OmType, Operand,
    ProjectElem, Rvalue, Statement, SwitchValue, Terminator, TypeId, UnwindAction,
};
use omlua_lua_backend::{LuaBackendProfile, LuaDialect, lower_program};
use omlua_lua_ir::{
    BackendRequirements, LirBinaryOp, LirBlock, LirBlockId, LirExpression, LirFunction,
    LirFunctionId, LirLocal, LirLocalId, LirProgram, LirStatement, LirTerminator, LirValue,
    LirValueKind, RuntimeHelper,
};

#[test]
fn lua54_profile_records_the_reference_runtime_model() {
    let profile = LuaBackendProfile::lua54();
    assert_eq!(profile.dialect(), LuaDialect::Lua54);
    assert_eq!(profile.numeric().integer_bits, 64);
    assert_eq!(profile.numeric().float_bits, 64);
    assert!(profile.control_flow().label_jumps);
    assert!(profile.operators().native_bitwise);
}

#[test]
fn lowers_structs_and_shared_references_to_packed_tables() {
    let point = TypeId::new(0);
    let program = OmProgram {
        entry: FunctionId::new(0),
        structs: vec![OmStruct {
            id: point,
            name: "Point".to_owned(),
            fields: vec![
                OmField {
                    id: FieldId::new(0),
                    name: "x".to_owned(),
                    ty: OmType::I32,
                },
                OmField {
                    id: FieldId::new(1),
                    name: "y".to_owned(),
                    ty: OmType::I32,
                },
            ],
        }],
        enums: vec![],
        functions: vec![OmFunction {
            id: FunctionId::new(0),
            name: "main".to_owned(),
            return_type: OmType::Unit,
            parameters: vec![],
            locals: vec![
                om_local(0, OmType::Unit, LocalKind::Return),
                om_local(1, OmType::Struct(point), LocalKind::Temporary),
                om_local(2, OmType::SharedRef(point), LocalKind::Temporary),
                om_local(3, OmType::I32, LocalKind::Temporary),
            ],
            blocks: vec![OmBlock {
                id: BlockId::new(0),
                statements: vec![
                    Statement::Assign {
                        destination: LocalId::new(1),
                        value: Rvalue::Struct {
                            ty: point,
                            fields: vec![
                                Operand::Constant(Constant::I32(20)),
                                Operand::Constant(Constant::I32(22)),
                            ],
                        },
                    },
                    Statement::Assign {
                        destination: LocalId::new(2),
                        value: Rvalue::SharedBorrow {
                            source: Operand::Copy(LocalId::new(1)),
                        },
                    },
                    Statement::Assign {
                        destination: LocalId::new(3),
                        value: Rvalue::Use(Operand::Project {
                            base: LocalId::new(2),
                            path: vec![ProjectElem::Deref, ProjectElem::Field(FieldId::new(1))],
                            moved: false,
                        }),
                    },
                ],
                terminator: Terminator::Return,
            }],
        }],
    };

    let lir = lower_program(&program, &LuaBackendProfile::lua54()).unwrap();
    assert_eq!(
        lir.functions[0].locals[0].kind,
        LirValueKind::Table(vec![LirValueKind::Integer, LirValueKind::Integer])
    );
    assert_eq!(
        lir.functions[0].blocks[0].statements,
        vec![
            LirStatement::Assign {
                destination: LirLocalId::new(1),
                value: LirExpression::Table {
                    fields: vec![
                        LirExpression::Value(LirValue::Integer(20)),
                        LirExpression::Value(LirValue::Integer(22)),
                    ],
                },
            },
            LirStatement::Assign {
                destination: LirLocalId::new(2),
                value: LirExpression::Value(LirValue::Local(LirLocalId::new(1))),
            },
            LirStatement::Assign {
                destination: LirLocalId::new(3),
                value: LirExpression::TableGet {
                    table: Box::new(LirExpression::Value(LirValue::Local(LirLocalId::new(2)))),
                    index: 2,
                    result: LirValueKind::Integer,
                },
            },
        ]
    );

    let mut missing_type = program.clone();
    missing_type.functions[0].locals[1].ty = OmType::Struct(TypeId::new(9));
    assert_eq!(
        lower_program(&missing_type, &LuaBackendProfile::lua54())
            .unwrap_err()
            .to_string(),
        concat!(
            "error[OMLUA0002]: local type references missing structure @9\n",
            "  in function `main`",
        )
    );

    let mut invalid_fields = program.clone();
    invalid_fields.structs[0].fields[0].id = FieldId::new(1);
    assert_eq!(
        lower_program(&invalid_fields, &LuaBackendProfile::lua54())
            .unwrap_err()
            .to_string(),
        "error[OMLUA0002]: structure @0 has non-contiguous field identifier .1"
    );

    let mut recursive = program.clone();
    recursive.structs[0].fields[0].ty = OmType::Struct(point);
    assert_eq!(
        lower_program(&recursive, &LuaBackendProfile::lua54())
            .unwrap_err()
            .to_string(),
        "error[OMLUA0002]: structure definitions contain a by-value cycle through @0"
    );
}

#[test]
fn lowers_checked_add_and_assert_to_explicit_lir() {
    let actual = lower_program(&checked_add_program(), &LuaBackendProfile::lua54()).unwrap();
    assert_eq!(actual, expected_checked_add_lir());
}

#[test]
fn requests_division_and_remainder_helpers_in_dependency_order() {
    let program = lower_program(&division_program(), &LuaBackendProfile::lua54()).unwrap();
    assert_eq!(
        program.helpers,
        vec![RuntimeHelper::I32DivTrunc, RuntimeHelper::I32Rem]
    );

    let statements = &program.functions[0].blocks[0].statements;
    assert!(matches!(
        &statements[0],
        LirStatement::Assign {
            value: LirExpression::RuntimeCall {
                helper: RuntimeHelper::I32DivTrunc,
                ..
            },
            ..
        }
    ));
    assert!(matches!(
        &statements[1],
        LirStatement::Assign {
            value: LirExpression::RuntimeCall {
                helper: RuntimeHelper::I32Rem,
                ..
            },
            ..
        }
    ));
}

#[test]
fn rejects_cleanup_unwind_edges_with_context() {
    let error = lower_program(&cleanup_unwind_program(), &LuaBackendProfile::lua54()).unwrap_err();
    assert_eq!(
        error.to_string(),
        concat!(
            "error[OMLUA0002]: cleanup unwind edges are not supported by backend `lua54`\n",
            "  in function `main`, basic block bb0",
        )
    );
}

#[test]
fn lowers_boolean_switches_to_boolean_branches() {
    let program = OmProgram {
        entry: FunctionId::new(0),
        structs: vec![],
        enums: vec![],
        functions: vec![OmFunction {
            id: FunctionId::new(0),
            name: "choose".to_owned(),
            return_type: OmType::Unit,
            parameters: vec![LocalId::new(1)],
            locals: vec![
                om_local(0, OmType::Unit, LocalKind::Return),
                om_local(1, OmType::Bool, LocalKind::Parameter),
            ],
            blocks: vec![
                OmBlock {
                    id: BlockId::new(0),
                    statements: Vec::new(),
                    terminator: Terminator::SwitchInt {
                        discriminant: Operand::Copy(LocalId::new(1)),
                        targets: vec![(SwitchValue(0), BlockId::new(2))],
                        otherwise: BlockId::new(1),
                    },
                },
                OmBlock {
                    id: BlockId::new(1),
                    statements: Vec::new(),
                    terminator: Terminator::Return,
                },
                OmBlock {
                    id: BlockId::new(2),
                    statements: Vec::new(),
                    terminator: Terminator::Return,
                },
            ],
        }],
    };

    let lir = lower_program(&program, &LuaBackendProfile::lua54()).unwrap();
    assert_eq!(
        lir.functions[0].blocks[0].terminator,
        LirTerminator::Branch {
            condition: LirExpression::Value(LirValue::Local(LirLocalId::new(1))),
            if_true: LirBlockId::new(1),
            if_false: LirBlockId::new(2),
        }
    );
}

#[test]
fn decodes_signed_i32_switch_values_from_their_rustc_bits() {
    let program = OmProgram {
        entry: FunctionId::new(0),
        structs: vec![],
        enums: vec![],
        functions: vec![OmFunction {
            id: FunctionId::new(0),
            name: "choose".to_owned(),
            return_type: OmType::Unit,
            parameters: vec![LocalId::new(1)],
            locals: vec![
                om_local(0, OmType::Unit, LocalKind::Return),
                om_local(1, OmType::I32, LocalKind::Parameter),
            ],
            blocks: vec![
                OmBlock {
                    id: BlockId::new(0),
                    statements: Vec::new(),
                    terminator: Terminator::SwitchInt {
                        discriminant: Operand::Copy(LocalId::new(1)),
                        targets: vec![(SwitchValue(u128::from(u32::MAX)), BlockId::new(1))],
                        otherwise: BlockId::new(2),
                    },
                },
                OmBlock {
                    id: BlockId::new(1),
                    statements: Vec::new(),
                    terminator: Terminator::Return,
                },
                OmBlock {
                    id: BlockId::new(2),
                    statements: Vec::new(),
                    terminator: Terminator::Return,
                },
            ],
        }],
    };

    let lir = lower_program(&program, &LuaBackendProfile::lua54()).unwrap();
    assert_eq!(
        lir.functions[0].blocks[0].terminator,
        LirTerminator::Switch {
            discriminant: LirExpression::Value(LirValue::Local(LirLocalId::new(1))),
            targets: vec![(-1, LirBlockId::new(1))],
            otherwise: LirBlockId::new(2),
        }
    );
}

fn division_program() -> OmProgram {
    OmProgram {
        entry: FunctionId::new(0),
        structs: vec![],
        enums: vec![],
        functions: vec![OmFunction {
            id: FunctionId::new(0),
            name: "calculate".to_owned(),
            return_type: OmType::I32,
            parameters: vec![LocalId::new(1), LocalId::new(2)],
            locals: vec![
                om_local(0, OmType::I32, LocalKind::Return),
                om_local(1, OmType::I32, LocalKind::Parameter),
                om_local(2, OmType::I32, LocalKind::Parameter),
                om_local(3, OmType::I32, LocalKind::Temporary),
                om_local(4, OmType::I32, LocalKind::Temporary),
            ],
            blocks: vec![OmBlock {
                id: BlockId::new(0),
                statements: vec![
                    Statement::Assign {
                        destination: LocalId::new(3),
                        value: Rvalue::Binary {
                            op: BinaryOp::Div,
                            left: Operand::Copy(LocalId::new(1)),
                            right: Operand::Copy(LocalId::new(2)),
                        },
                    },
                    Statement::Assign {
                        destination: LocalId::new(4),
                        value: Rvalue::Binary {
                            op: BinaryOp::Rem,
                            left: Operand::Copy(LocalId::new(1)),
                            right: Operand::Copy(LocalId::new(2)),
                        },
                    },
                    Statement::Assign {
                        destination: LocalId::new(0),
                        value: Rvalue::Use(Operand::Move(LocalId::new(4))),
                    },
                ],
                terminator: Terminator::Return,
            }],
        }],
    }
}

fn cleanup_unwind_program() -> OmProgram {
    OmProgram {
        entry: FunctionId::new(0),
        structs: vec![],
        enums: vec![],
        functions: vec![OmFunction {
            id: FunctionId::new(0),
            name: "main".to_owned(),
            return_type: OmType::Unit,
            parameters: Vec::new(),
            locals: vec![om_local(0, OmType::Unit, LocalKind::Return)],
            blocks: vec![OmBlock {
                id: BlockId::new(0),
                statements: Vec::new(),
                terminator: Terminator::Call {
                    callee: FunctionId::new(1),
                    arguments: Vec::new(),
                    destination: LocalId::new(0),
                    target: BlockId::new(0),
                    unwind: UnwindAction::Cleanup(BlockId::new(1)),
                },
            }],
        }],
    }
}

fn checked_add_program() -> OmProgram {
    OmProgram {
        entry: FunctionId::new(0),
        structs: vec![],
        enums: vec![],
        functions: vec![OmFunction {
            id: FunctionId::new(0),
            name: "main".to_owned(),
            return_type: OmType::I32,
            parameters: vec![LocalId::new(1)],
            locals: vec![
                om_local(0, OmType::I32, LocalKind::Return),
                om_local(1, OmType::I32, LocalKind::Parameter),
                om_local(2, OmType::I32, LocalKind::CheckedValue),
                om_local(3, OmType::Bool, LocalKind::CheckedOverflow),
            ],
            blocks: vec![
                OmBlock {
                    id: BlockId::new(0),
                    statements: vec![Statement::CheckedBinary {
                        value: LocalId::new(2),
                        overflow: LocalId::new(3),
                        op: CheckedBinaryOp::Add,
                        left: Operand::Copy(LocalId::new(1)),
                        right: Operand::Constant(Constant::I32(1)),
                    }],
                    terminator: Terminator::Assert {
                        condition: Operand::Move(LocalId::new(3)),
                        expected: false,
                        kind: AssertKind::Overflow {
                            op: BinaryOp::Add,
                            left: Operand::Copy(LocalId::new(1)),
                            right: Operand::Constant(Constant::I32(1)),
                        },
                        target: BlockId::new(1),
                        unwind: UnwindAction::Continue,
                    },
                },
                OmBlock {
                    id: BlockId::new(1),
                    statements: vec![Statement::Assign {
                        destination: LocalId::new(0),
                        value: Rvalue::Use(Operand::Move(LocalId::new(2))),
                    }],
                    terminator: Terminator::Return,
                },
            ],
        }],
    }
}

fn om_local(index: u32, ty: OmType, kind: LocalKind) -> OmLocal {
    OmLocal {
        id: LocalId::new(index),
        ty,
        kind,
    }
}

fn expected_checked_add_lir() -> LirProgram {
    let local = |index| LirExpression::Value(LirValue::Local(LirLocalId::new(index)));
    let integer = |value| LirExpression::Value(LirValue::Integer(value));
    let binary = |op, left, right| LirExpression::Binary {
        op,
        left: Box::new(left),
        right: Box::new(right),
    };

    LirProgram {
        entry: LirFunctionId::new(0),
        requirements: BackendRequirements {
            minimum_integer_bits: 64,
            label_jumps: true,
            native_bitwise: false,
        },
        helpers: Vec::new(),
        functions: vec![LirFunction {
            id: LirFunctionId::new(0),
            entry: LirBlockId::new(0),
            parameters: vec![LirLocalId::new(1)],
            return_local: Some(LirLocalId::new(0)),
            locals: vec![
                lir_local(0, LirValueKind::Integer, false),
                lir_local(1, LirValueKind::Integer, true),
                lir_local(2, LirValueKind::Integer, false),
                lir_local(3, LirValueKind::Bool, false),
            ],
            blocks: vec![
                LirBlock {
                    id: LirBlockId::new(0),
                    statements: vec![
                        LirStatement::Assign {
                            destination: LirLocalId::new(2),
                            value: binary(LirBinaryOp::Add, local(1), integer(1)),
                        },
                        LirStatement::Assign {
                            destination: LirLocalId::new(3),
                            value: binary(
                                LirBinaryOp::Or,
                                binary(LirBinaryOp::Lt, local(2), integer(i32::MIN.into())),
                                binary(LirBinaryOp::Gt, local(2), integer(i32::MAX.into())),
                            ),
                        },
                    ],
                    terminator: LirTerminator::Branch {
                        condition: local(3),
                        if_true: LirBlockId::new(2),
                        if_false: LirBlockId::new(1),
                    },
                },
                LirBlock {
                    id: LirBlockId::new(1),
                    statements: vec![LirStatement::Assign {
                        destination: LirLocalId::new(0),
                        value: local(2),
                    }],
                    terminator: LirTerminator::Return {
                        value: Some(local(0)),
                    },
                },
                LirBlock {
                    id: LirBlockId::new(2),
                    statements: Vec::new(),
                    terminator: LirTerminator::Raise {
                        message: "attempt to add with overflow".to_owned(),
                    },
                },
            ],
        }],
    }
}

fn lir_local(index: u32, kind: LirValueKind, parameter: bool) -> LirLocal {
    LirLocal {
        id: LirLocalId::new(index),
        kind,
        parameter,
    }
}
