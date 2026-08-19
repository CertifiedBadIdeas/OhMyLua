use omlua_ir::{
    BlockId, FieldId, FunctionId, LocalId, LocalKind, OmBlock, OmFunction, OmLocal, OmType,
    Operand, ProjectElem, Rvalue, Statement, SwitchValue, Terminator, TypeId, VariantId,
};
use rustc_middle::ty::{self, Ty, TyCtxt};

use crate::LowerError;

use super::types::{normalize_core_enum_path, TypeRegistry};

/// A call to a core `Try`/`FromResidual` method that the adapter desugars structurally
/// instead of registering as an external callee. The carried types are the instantiated
/// self/flow/residual types observed at the call site.
pub(super) enum TryCall<'tcx> {
    OptionBranch { option: Ty<'tcx>, flow: Ty<'tcx> },
    ResultBranch { result: Ty<'tcx>, flow: Ty<'tcx> },
    OptionFromResidual { option: Ty<'tcx> },
    ResultFromResidual { result: Ty<'tcx>, residual: Ty<'tcx> },
}

impl TryCall<'_> {
    pub(super) fn name(&self) -> String {
        let render = |ty: Ty<'_>| normalize_core_enum_path(&ty.to_string());
        match self {
            TryCall::OptionBranch { option, .. } => {
                format!("__omlua_option_branch<{}>", render(adt_argument(*option, 0)))
            }
            TryCall::ResultBranch { result, .. } => format!(
                "__omlua_result_branch<{}, {}>",
                render(adt_argument(*result, 0)),
                render(adt_argument(*result, 1))
            ),
            TryCall::OptionFromResidual { option } => {
                format!(
                    "__omlua_option_from_residual<{}>",
                    render(adt_argument(*option, 0))
                )
            }
            TryCall::ResultFromResidual { result, .. } => format!(
                "__omlua_result_from_residual<{}, {}>",
                render(adt_argument(*result, 0)),
                render(adt_argument(*result, 1))
            ),
        }
    }
}

fn adt_argument(ty: Ty<'_>, index: usize) -> Ty<'_> {
    let ty::Adt(_, arguments) = ty.kind() else {
        unreachable!("Try calls are only classified for Option and Result self types");
    };
    arguments.type_at(index)
}

pub(super) fn synthesize_try_helper<'tcx>(
    tcx: TyCtxt<'tcx>,
    id: FunctionId,
    name: &str,
    call: &TryCall<'tcx>,
    types: &mut TypeRegistry,
) -> Result<OmFunction, LowerError> {
    match call {
        TryCall::OptionBranch { option, flow } => {
            option_branch(tcx, id, name, types, *option, *flow)
        }
        TryCall::ResultBranch { result, flow } => {
            result_branch(tcx, id, name, types, *result, *flow)
        }
        TryCall::OptionFromResidual { option } => {
            option_from_residual(tcx, id, name, types, *option)
        }
        TryCall::ResultFromResidual { result, residual } => {
            result_from_residual(tcx, id, name, types, *result, *residual)
        }
    }
}

fn om_enum<'tcx>(
    tcx: TyCtxt<'tcx>,
    types: &mut TypeRegistry,
    ty: Ty<'tcx>,
) -> Result<TypeId, LowerError> {
    match types.lower_type(tcx, ty)? {
        OmType::Enum(id) => Ok(id),
        _ => Err(LowerError::program(format!(
            "Try helper type `{ty}` did not lower to an enum"
        ))),
    }
}

fn option_branch<'tcx>(
    tcx: TyCtxt<'tcx>,
    id: FunctionId,
    name: &str,
    types: &mut TypeRegistry,
    option: Ty<'tcx>,
    flow: Ty<'tcx>,
) -> Result<OmFunction, LowerError> {
    let option_id = om_enum(tcx, types, option)?;
    let flow_id = om_enum(tcx, types, flow)?;
    let residual_id = om_enum(tcx, types, adt_argument(flow, 0))?;

    let mut locals = Vec::new();
    let returned = push(OmType::Enum(flow_id), LocalKind::Return, &mut locals);
    let parameter = push(OmType::Enum(option_id), LocalKind::Parameter, &mut locals);
    let discriminant = push(OmType::I32, LocalKind::Discriminant, &mut locals);
    let continued = push(OmType::Enum(flow_id), LocalKind::Temporary, &mut locals);
    let residual = push(OmType::Enum(residual_id), LocalKind::Temporary, &mut locals);
    let broken = push(OmType::Enum(flow_id), LocalKind::Temporary, &mut locals);

    Ok(OmFunction {
        id,
        name: name.to_owned(),
        return_type: OmType::Enum(flow_id),
        parameters: vec![parameter],
        locals,
        blocks: vec![
            OmBlock {
                id: BlockId::new(0),
                statements: vec![Statement::Assign {
                    destination: discriminant,
                    value: Rvalue::Discriminant {
                        source: Operand::Copy(parameter),
                    },
                }],
                terminator: Terminator::SwitchInt {
                    discriminant: Operand::Move(discriminant),
                    targets: vec![
                        (SwitchValue(0), BlockId::new(2)),
                        (SwitchValue(1), BlockId::new(1)),
                    ],
                    otherwise: BlockId::new(3),
                },
            },
            OmBlock {
                id: BlockId::new(1),
                statements: vec![
                    Statement::Assign {
                        destination: continued,
                        value: Rvalue::Variant {
                            ty: flow_id,
                            variant: VariantId::new(0),
                            fields: vec![Operand::Project {
                                base: parameter,
                                path: vec![
                                    ProjectElem::Downcast(VariantId::new(1)),
                                    ProjectElem::Field(FieldId::new(0)),
                                ],
                                moved: false,
                            }],
                        },
                    },
                    Statement::Assign {
                        destination: returned,
                        value: Rvalue::Use(Operand::Move(continued)),
                    },
                ],
                terminator: Terminator::Return,
            },
            OmBlock {
                id: BlockId::new(2),
                statements: vec![
                    Statement::Assign {
                        destination: residual,
                        value: Rvalue::Variant {
                            ty: residual_id,
                            variant: VariantId::new(0),
                            fields: vec![],
                        },
                    },
                    Statement::Assign {
                        destination: broken,
                        value: Rvalue::Variant {
                            ty: flow_id,
                            variant: VariantId::new(1),
                            fields: vec![Operand::Move(residual)],
                        },
                    },
                    Statement::Assign {
                        destination: returned,
                        value: Rvalue::Use(Operand::Move(broken)),
                    },
                ],
                terminator: Terminator::Return,
            },
            OmBlock {
                id: BlockId::new(3),
                statements: vec![],
                terminator: Terminator::Unreachable,
            },
        ],
    })
}

fn result_branch<'tcx>(
    tcx: TyCtxt<'tcx>,
    id: FunctionId,
    name: &str,
    types: &mut TypeRegistry,
    result: Ty<'tcx>,
    flow: Ty<'tcx>,
) -> Result<OmFunction, LowerError> {
    let result_id = om_enum(tcx, types, result)?;
    let flow_id = om_enum(tcx, types, flow)?;
    let residual_id = om_enum(tcx, types, adt_argument(flow, 0))?;

    let mut locals = Vec::new();
    let returned = push(OmType::Enum(flow_id), LocalKind::Return, &mut locals);
    let parameter = push(OmType::Enum(result_id), LocalKind::Parameter, &mut locals);
    let discriminant = push(OmType::I32, LocalKind::Discriminant, &mut locals);
    let continued = push(OmType::Enum(flow_id), LocalKind::Temporary, &mut locals);
    let residual = push(OmType::Enum(residual_id), LocalKind::Temporary, &mut locals);
    let broken = push(OmType::Enum(flow_id), LocalKind::Temporary, &mut locals);

    Ok(OmFunction {
        id,
        name: name.to_owned(),
        return_type: OmType::Enum(flow_id),
        parameters: vec![parameter],
        locals,
        blocks: vec![
            OmBlock {
                id: BlockId::new(0),
                statements: vec![Statement::Assign {
                    destination: discriminant,
                    value: Rvalue::Discriminant {
                        source: Operand::Copy(parameter),
                    },
                }],
                terminator: Terminator::SwitchInt {
                    discriminant: Operand::Move(discriminant),
                    targets: vec![
                        (SwitchValue(0), BlockId::new(1)),
                        (SwitchValue(1), BlockId::new(2)),
                    ],
                    otherwise: BlockId::new(3),
                },
            },
            OmBlock {
                id: BlockId::new(1),
                statements: vec![
                    Statement::Assign {
                        destination: continued,
                        value: Rvalue::Variant {
                            ty: flow_id,
                            variant: VariantId::new(0),
                            fields: vec![Operand::Project {
                                base: parameter,
                                path: vec![
                                    ProjectElem::Downcast(VariantId::new(0)),
                                    ProjectElem::Field(FieldId::new(0)),
                                ],
                                moved: false,
                            }],
                        },
                    },
                    Statement::Assign {
                        destination: returned,
                        value: Rvalue::Use(Operand::Move(continued)),
                    },
                ],
                terminator: Terminator::Return,
            },
            OmBlock {
                id: BlockId::new(2),
                statements: vec![
                    Statement::Assign {
                        destination: residual,
                        value: Rvalue::Variant {
                            ty: residual_id,
                            variant: VariantId::new(1),
                            fields: vec![Operand::Project {
                                base: parameter,
                                path: vec![
                                    ProjectElem::Downcast(VariantId::new(1)),
                                    ProjectElem::Field(FieldId::new(0)),
                                ],
                                moved: false,
                            }],
                        },
                    },
                    Statement::Assign {
                        destination: broken,
                        value: Rvalue::Variant {
                            ty: flow_id,
                            variant: VariantId::new(1),
                            fields: vec![Operand::Move(residual)],
                        },
                    },
                    Statement::Assign {
                        destination: returned,
                        value: Rvalue::Use(Operand::Move(broken)),
                    },
                ],
                terminator: Terminator::Return,
            },
            OmBlock {
                id: BlockId::new(3),
                statements: vec![],
                terminator: Terminator::Unreachable,
            },
        ],
    })
}

fn option_from_residual<'tcx>(
    tcx: TyCtxt<'tcx>,
    id: FunctionId,
    name: &str,
    types: &mut TypeRegistry,
    option: Ty<'tcx>,
) -> Result<OmFunction, LowerError> {
    let option_id = om_enum(tcx, types, option)?;

    let mut locals = Vec::new();
    let returned = push(OmType::Enum(option_id), LocalKind::Return, &mut locals);

    Ok(OmFunction {
        id,
        name: name.to_owned(),
        return_type: OmType::Enum(option_id),
        parameters: vec![],
        locals,
        blocks: vec![OmBlock {
            id: BlockId::new(0),
            statements: vec![Statement::Assign {
                destination: returned,
                value: Rvalue::Variant {
                    ty: option_id,
                    variant: VariantId::new(0),
                    fields: vec![],
                },
            }],
            terminator: Terminator::Return,
        }],
    })
}

fn result_from_residual<'tcx>(
    tcx: TyCtxt<'tcx>,
    id: FunctionId,
    name: &str,
    types: &mut TypeRegistry,
    result: Ty<'tcx>,
    residual: Ty<'tcx>,
) -> Result<OmFunction, LowerError> {
    let result_id = om_enum(tcx, types, result)?;
    let residual_id = om_enum(tcx, types, residual)?;

    let mut locals = Vec::new();
    let returned = push(OmType::Enum(result_id), LocalKind::Return, &mut locals);
    let parameter = push(OmType::Enum(residual_id), LocalKind::Parameter, &mut locals);

    Ok(OmFunction {
        id,
        name: name.to_owned(),
        return_type: OmType::Enum(result_id),
        parameters: vec![parameter],
        locals,
        blocks: vec![OmBlock {
            id: BlockId::new(0),
            statements: vec![Statement::Assign {
                destination: returned,
                value: Rvalue::Variant {
                    ty: result_id,
                    variant: VariantId::new(1),
                    fields: vec![Operand::Project {
                        base: parameter,
                        path: vec![
                            ProjectElem::Downcast(VariantId::new(1)),
                            ProjectElem::Field(FieldId::new(0)),
                        ],
                        moved: true,
                    }],
                },
            }],
            terminator: Terminator::Return,
        }],
    })
}

fn push(ty: OmType, kind: LocalKind, locals: &mut Vec<OmLocal>) -> LocalId {
    let id = LocalId::new(locals.len() as u32);
    locals.push(OmLocal { id, ty, kind });
    id
}
