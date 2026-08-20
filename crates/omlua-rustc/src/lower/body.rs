use std::collections::HashSet;

use omlua_ir::{
    AssertKind, BinaryOp, BlockId, CheckedBinaryOp, Constant, FieldId, FunctionId, LocalId,
    LocalKind, OmBlock, OmFunction, OmLocal, OmType, Operand, Place as OmPlace, ProjectElem,
    RefKind, Rvalue, Statement, SwitchValue, Terminator, TypeId, UnaryOp, UnwindAction, VariantId,
};
use rustc_index::Idx;
use rustc_middle::mir::{
    self, AggregateKind, AssertKind as MirAssertKind, BasicBlock, BinOp as MirBinOp, Body,
    BorrowKind, Local, Operand as MirOperand, Place, PlaceElem, RETURN_PLACE, Rvalue as MirRvalue,
    StatementKind, TerminatorKind, UnOp as MirUnOp, UnwindAction as MirUnwindAction,
    UnwindTerminateReason,
};
use rustc_middle::ty::{self, IntTy, Ty, TyCtxt, TypingEnv};
use rustc_span::def_id::DefId;

use crate::LowerError;

use super::program::FunctionRegistry;
use super::synthetic_helper::SyntheticCall;
use super::types::{core_range_name, core_try_enum_name};

pub(super) fn lower_function(
    tcx: TyCtxt<'_>,
    def_id: DefId,
    id: FunctionId,
    registry: &mut FunctionRegistry,
) -> Result<OmFunction, LowerError> {
    let name = tcx.def_path_str(def_id);
    let body = tcx.optimized_mir(def_id);
    if body.is_polymorphic {
        return Err(LowerError::function(
            &name,
            "polymorphic MIR is not supported",
        ));
    }
    BodyLowerer::new(tcx, body, name, registry)?.lower(id)
}

#[derive(Clone, Copy)]
enum LocalMapping {
    Scalar(LocalId),
    CheckedPair { value: LocalId, overflow: LocalId },
}

struct BodyLowerer<'a, 'tcx> {
    tcx: TyCtxt<'tcx>,
    body: &'tcx Body<'tcx>,
    name: String,
    registry: &'a mut FunctionRegistry,
    locals: Vec<OmLocal>,
    local_map: Vec<LocalMapping>,
    synthetic_blocks: Vec<OmBlock>,
    next_synthetic_block: u32,
}

impl<'a, 'tcx> BodyLowerer<'a, 'tcx> {
    fn new(
        tcx: TyCtxt<'tcx>,
        body: &'tcx Body<'tcx>,
        name: String,
        registry: &'a mut FunctionRegistry,
    ) -> Result<Self, LowerError> {
        let checked_locals = find_checked_locals(body);
        let discriminant_locals = find_discriminant_locals(body);
        let mut locals = Vec::new();
        let mut local_map = Vec::with_capacity(body.local_decls.len());

        for (local, declaration) in body.local_decls.iter_enumerated() {
            if checked_locals.contains(&local) {
                validate_checked_pair_type(declaration.ty).map_err(|detail| {
                    LowerError::function(&name, format!("local _{}: {detail}", local.index()))
                })?;
                let value = push_local(&mut locals, OmType::I32, LocalKind::CheckedValue, &name)?;
                let overflow =
                    push_local(&mut locals, OmType::Bool, LocalKind::CheckedOverflow, &name)?;
                local_map.push(LocalMapping::CheckedPair { value, overflow });
            } else if discriminant_locals.contains(&local) {
                if !matches!(declaration.ty.kind(), ty::Int(_)) {
                    return Err(LowerError::function(
                        &name,
                        format!(
                            "local _{}: discriminant type `{}` is not supported",
                            local.index(),
                            declaration.ty
                        ),
                    ));
                }
                let id = push_local(&mut locals, OmType::I32, LocalKind::Discriminant, &name)?;
                local_map.push(LocalMapping::Scalar(id));
            } else {
                let ty = registry
                    .types
                    .lower_type(tcx, declaration.ty)
                    .map_err(|error| {
                        LowerError::function(
                            &name,
                            format!("local _{}: {}", local.index(), error.detail()),
                        )
                    })?;
                let kind = if local == RETURN_PLACE {
                    LocalKind::Return
                } else if local.index() <= body.arg_count {
                    LocalKind::Parameter
                } else {
                    LocalKind::Temporary
                };
                let id = push_local(&mut locals, ty, kind, &name)?;
                local_map.push(LocalMapping::Scalar(id));
            }
        }

        let next_synthetic_block = u32::try_from(body.basic_blocks.len())
            .map_err(|_| LowerError::function(&name, "basic block count exceeds OMIR limits"))?;

        Ok(Self {
            tcx,
            body,
            name,
            registry,
            locals,
            local_map,
            synthetic_blocks: Vec::new(),
            next_synthetic_block,
        })
    }

    fn lower(mut self, id: FunctionId) -> Result<OmFunction, LowerError> {
        let return_local = self.scalar_local(RETURN_PLACE)?;
        let return_type = self.locals[return_local.index() as usize].ty;
        let parameters = (1..=self.body.arg_count)
            .map(|index| self.scalar_local(Local::new(index)))
            .collect::<Result<Vec<_>, _>>()?;

        let mut blocks = Vec::with_capacity(self.body.basic_blocks.len());
        for (block, data) in self.body.basic_blocks.iter_enumerated() {
            blocks.push(self.lower_block(block, data)?);
        }
        blocks.append(&mut self.synthetic_blocks);

        Ok(OmFunction {
            id,
            name: self.name,
            return_type,
            parameters,
            locals: self.locals,
            blocks,
        })
    }

    fn lower_block(
        &mut self,
        block: BasicBlock,
        data: &mir::BasicBlockData<'tcx>,
    ) -> Result<OmBlock, LowerError> {
        let mut statements = Vec::new();
        for statement in &data.statements {
            if let Some(statement) = self.lower_statement(block, &statement.kind)? {
                statements.push(statement);
            }
        }
        let terminator = match self.try_lower_range_next(block, &data.terminator().kind)? {
            Some((extra, terminator)) => {
                statements.extend(extra);
                terminator
            }
            None => self.lower_terminator(block, &data.terminator().kind)?,
        };
        Ok(OmBlock {
            id: block_id(block, &self.name)?,
            statements,
            terminator,
        })
    }

    fn lower_statement(
        &mut self,
        block: BasicBlock,
        kind: &StatementKind<'tcx>,
    ) -> Result<Option<Statement>, LowerError> {
        match kind {
            StatementKind::Assign(assignment) => {
                let (place, value) = &**assignment;
                if let Some(op) = checked_binary(value) {
                    let LocalMapping::CheckedPair {
                        value: destination,
                        overflow,
                    } = self.mapping_for_bare_place(place, block)?
                    else {
                        return Err(self.block_error(
                            block,
                            "checked arithmetic destination is not a compiler-generated pair",
                        ));
                    };
                    let MirRvalue::BinaryOp(_, operands) = value else {
                        unreachable!()
                    };
                    return Ok(Some(Statement::CheckedBinary {
                        value: destination,
                        overflow,
                        op,
                        left: self.lower_operand(block, &operands.0)?,
                        right: self.lower_operand(block, &operands.1)?,
                    }));
                }

                let destination = self.lower_place(block, place)?;
                let value = self.lower_rvalue(
                    block,
                    value,
                    place.ty(&self.body.local_decls, self.tcx).ty,
                )?;
                if destination.path.is_empty() {
                    Ok(Some(Statement::Assign {
                        destination: destination.base,
                        value,
                    }))
                } else {
                    Ok(Some(Statement::Store { destination, value }))
                }
            }
            StatementKind::StorageLive(_) | StatementKind::StorageDead(_) | StatementKind::Nop => {
                Ok(None)
            }
            other => {
                Err(self.block_error(block, format!("MIR statement `{other:?}` is not supported")))
            }
        }
    }

    fn lower_rvalue(
        &mut self,
        block: BasicBlock,
        value: &MirRvalue<'tcx>,
        destination_ty: Ty<'tcx>,
    ) -> Result<Rvalue, LowerError> {
        match value {
            MirRvalue::Use(operand, _) => Ok(Rvalue::Use(self.lower_operand(block, operand)?)),
            MirRvalue::Discriminant(place) => Ok(Rvalue::Discriminant {
                source: self.lower_operand_place(block, place, false)?,
            }),
            MirRvalue::UnaryOp(op, operand) => {
                let operand_ty = operand.ty(&self.body.local_decls, self.tcx);
                Ok(Rvalue::Unary {
                    op: lower_unary(*op, operand_ty).ok_or_else(|| {
                        self.block_error(
                            block,
                            format!(
                                "unary operation `{op:?}` is not supported for `{operand_ty}`"
                            ),
                        )
                    })?,
                    operand: self.lower_operand(block, operand)?,
                })
            }
            MirRvalue::BinaryOp(op, operands) => Ok(Rvalue::Binary {
                op: lower_binary(
                    *op,
                    operands.0.ty(&self.body.local_decls, self.tcx),
                    operands.1.ty(&self.body.local_decls, self.tcx),
                )
                .ok_or_else(|| {
                    self.block_error(block, format!("binary operation `{op:?}` is not supported"))
                })?,
                left: self.lower_operand(block, &operands.0)?,
                right: self.lower_operand(block, &operands.1)?,
            }),
            MirRvalue::Aggregate(kind, operands) => {
                let AggregateKind::Adt(_, variant, _, _, _) = kind.as_ref() else {
                    return Err(self.block_error(block, "non-structure aggregate is not supported"));
                };
                let ty = self
                    .registry
                    .types
                    .lower_type(self.tcx, destination_ty)
                    .map_err(|error| self.block_error(block, error.detail()))?;
                let fields = operands
                    .iter()
                    .map(|operand| self.lower_operand(block, operand))
                    .collect::<Result<_, _>>()?;
                match ty {
                    OmType::Enum(id) => Ok(Rvalue::Variant {
                        ty: id,
                        variant: VariantId::new(variant.as_u32()),
                        fields,
                    }),
                    OmType::Struct(id) => {
                        if variant.as_usize() != 0 {
                            return Err(
                                self.block_error(block, "non-structure aggregate is not supported")
                            );
                        }
                        Ok(Rvalue::Struct { ty: id, fields })
                    }
                    _ => Err(self.block_error(
                        block,
                        "aggregate destination is not a structure or enum type",
                    )),
                }
            }
            MirRvalue::Ref(_, borrow_kind, place) => {
                let kind = match borrow_kind {
                    BorrowKind::Shared => RefKind::Shared,
                    BorrowKind::Mut { .. } => RefKind::Mutable,
                    other => {
                        return Err(self.block_error(
                            block,
                            format!("borrow kind `{other:?}` is not supported"),
                        ));
                    }
                };
                let source = self.lower_place(block, place)?;
                let source_ty = self.om_place_type(&source)?;
                let expected = self
                    .registry
                    .types
                    .lower_type(self.tcx, destination_ty)
                    .map_err(|error| self.block_error(block, error.detail()))?;
                let OmType::Ref {
                    kind: expected_kind,
                    target,
                } = expected
                else {
                    return Err(self.block_error(block, "borrow destination is not a reference"));
                };
                if expected_kind != kind || target.as_type() != source_ty {
                    return Err(self.block_error(
                        block,
                        "borrow source and destination reference types do not match",
                    ));
                }
                Ok(Rvalue::Borrow { kind, source })
            }
            other => {
                Err(self.block_error(block, format!("MIR rvalue `{other:?}` is not supported")))
            }
        }
    }

    fn lower_operand(
        &self,
        block: BasicBlock,
        operand: &MirOperand<'tcx>,
    ) -> Result<Operand, LowerError> {
        match operand {
            MirOperand::Copy(place) => self.lower_operand_place(block, place, false),
            MirOperand::Move(place) => self.lower_operand_place(block, place, true),
            MirOperand::Constant(constant) => {
                let ty = constant.const_.ty();
                let typing_env = TypingEnv::fully_monomorphized();
                let value = match ty.kind() {
                    ty::Tuple(fields) if fields.is_empty() => Constant::Unit,
                    ty::Bool => Constant::Bool(
                        constant
                            .const_
                            .try_eval_bool(self.tcx, typing_env)
                            .ok_or_else(|| {
                                self.block_error(block, "bool constant is not evaluable")
                            })?,
                    ),
                    ty::Int(IntTy::I32) => {
                        let bits = constant
                            .const_
                            .try_eval_bits(self.tcx, typing_env)
                            .ok_or_else(|| {
                                self.block_error(block, "i32 constant is not evaluable")
                            })?;
                        Constant::I32(bits as u32 as i32)
                    }
                    _ => {
                        return Err(self
                            .block_error(block, format!("constant type `{ty}` is not supported")));
                    }
                };
                Ok(Operand::Constant(value))
            }
            MirOperand::RuntimeChecks(_) => {
                Err(self.block_error(block, "runtime-check query operands are not supported"))
            }
        }
    }

    fn lower_operand_place(
        &self,
        block: BasicBlock,
        place: &Place<'tcx>,
        moved: bool,
    ) -> Result<Operand, LowerError> {
        let place = self.lower_place(block, place)?;
        if place.path.is_empty() {
            return Ok(if moved {
                Operand::Move(place.base)
            } else {
                Operand::Copy(place.base)
            });
        }
        Ok(Operand::Project {
            base: place.base,
            path: place.path,
            moved,
        })
    }

    fn lower_place(&self, block: BasicBlock, place: &Place<'tcx>) -> Result<OmPlace, LowerError> {
        if let LocalMapping::CheckedPair { value, overflow } = self.local_map[place.local.index()] {
            return match place.projection.as_ref() {
                [PlaceElem::Field(field, _)] if field.index() == 0 => Ok(OmPlace::local(value)),
                [PlaceElem::Field(field, _)] if field.index() == 1 => Ok(OmPlace::local(overflow)),
                _ => Err(self.block_error(
                    block,
                    "compiler-generated checked pair is only addressable through fields .0 and .1",
                )),
            };
        }

        let LocalMapping::Scalar(base) = self.local_map[place.local.index()] else {
            unreachable!()
        };
        let mut ty = self.locals[base.index() as usize].ty;
        let mut path = Vec::new();
        let mut downcast: Option<VariantId> = None;
        for projection in place.projection.as_ref() {
            match projection {
                PlaceElem::Deref => {
                    let OmType::Ref { target, .. } = ty else {
                        return Err(self.block_error(block, "dereference of a non-reference value"));
                    };
                    ty = target.as_type();
                    path.push(ProjectElem::Deref);
                }
                PlaceElem::Downcast(_name, variant) => {
                    if downcast.is_some() {
                        return Err(self.block_error(block, "nested downcast without a field access"));
                    }
                    let OmType::Enum(type_id) = ty else {
                        return Err(self.block_error(block, "downcast of a non-enum value"));
                    };
                    let variant = VariantId::new(variant.as_u32());
                    self.validate_variant(block, type_id, variant)?;
                    downcast = Some(variant);
                    path.push(ProjectElem::Downcast(variant));
                }
                PlaceElem::Field(field, _) => {
                    let definition_field = match (ty, downcast) {
                        (OmType::Struct(type_id), None) => {
                            self.struct_field(block, type_id, field.as_usize())?
                        }
                        (OmType::Enum(type_id), Some(variant)) => {
                            self.variant_field(block, type_id, variant, field.as_usize())?
                        }
                        (OmType::Enum(_), None) => {
                            return Err(self.block_error(
                                block,
                                "enum field access requires a variant downcast",
                            ));
                        }
                        _ => {
                            return Err(self.block_error(block, "field access on a non-aggregate value"));
                        }
                    };
                    path.push(ProjectElem::Field(FieldId::new(field.as_u32())));
                    ty = definition_field.ty;
                    downcast = None;
                }
                other => {
                    return Err(self.block_error(
                        block,
                        format!("place projection `{other:?}` is not supported"),
                    ));
                }
            }
        }
        if downcast.is_some() {
            return Err(self.block_error(block, "enum downcast without a field access"));
        }
        Ok(OmPlace { base, path })
    }

    fn om_place_type(&self, place: &OmPlace) -> Result<OmType, LowerError> {
        let mut ty = self.locals[place.base.index() as usize].ty;
        let mut downcast: Option<VariantId> = None;
        for element in &place.path {
            match element {
                ProjectElem::Deref => {
                    let OmType::Ref { target, .. } = ty else {
                        return Err(LowerError::function(
                            &self.name,
                            "dereference source is not a reference",
                        ));
                    };
                    ty = target.as_type();
                }
                ProjectElem::Downcast(variant) => {
                    let OmType::Enum(_) = ty else {
                        return Err(LowerError::function(
                            &self.name,
                            "downcast crosses a non-enum value",
                        ));
                    };
                    downcast = Some(*variant);
                }
                ProjectElem::Field(field) => {
                    let definition_field = match (ty, downcast) {
                        (OmType::Struct(type_id), None) => self
                            .registry
                            .types
                            .definition(type_id)
                            .and_then(|definition| definition.fields.get(field.index() as usize))
                            .filter(|definition| definition.id == *field)
                            .ok_or_else(|| {
                                LowerError::function(
                                    &self.name,
                                    format!("field .{field} does not exist in structure @{type_id}"),
                                )
                            })?,
                        (OmType::Enum(type_id), Some(variant)) => self
                            .registry
                            .types
                            .enum_definition(type_id)
                            .and_then(|definition| definition.variants.get(variant.index() as usize))
                            .filter(|definition| definition.id == variant)
                            .and_then(|variant| variant.fields.get(field.index() as usize))
                            .filter(|definition| definition.id == *field)
                            .ok_or_else(|| {
                                LowerError::function(
                                    &self.name,
                                    format!(
                                        "field .{field} does not exist in variant v{variant} of enum @{type_id}"
                                    ),
                                )
                            })?,
                        (OmType::Enum(_), None) => {
                            return Err(LowerError::function(
                                &self.name,
                                "enum field access requires a variant downcast",
                            ));
                        }
                        _ => {
                            return Err(LowerError::function(
                                &self.name,
                                "field projection crosses a non-aggregate value",
                            ));
                        }
                    };
                    ty = definition_field.ty;
                    downcast = None;
                }
            }
        }
        if downcast.is_some() {
            return Err(LowerError::function(
                &self.name,
                "projection ends with a downcast without a field access",
            ));
        }
        Ok(ty)
    }

    fn mapping_for_bare_place(
        &self,
        place: &Place<'tcx>,
        block: BasicBlock,
    ) -> Result<LocalMapping, LowerError> {
        if !place.projection.is_empty() {
            return Err(self.block_error(block, "checked arithmetic destination has a projection"));
        }
        Ok(self.local_map[place.local.index()])
    }

    fn scalar_local(&self, local: Local) -> Result<LocalId, LowerError> {
        match self.local_map[local.index()] {
            LocalMapping::Scalar(id) => Ok(id),
            LocalMapping::CheckedPair { .. } => Err(LowerError::function(
                &self.name,
                format!("local _{} cannot be used as a scalar", local.index()),
            )),
        }
    }

    fn try_lower_range_next(
        &mut self,
        block: BasicBlock,
        kind: &TerminatorKind<'tcx>,
    ) -> Result<Option<(Vec<Statement>, Terminator)>, LowerError> {
        let TerminatorKind::Call {
            func,
            args,
            destination,
            target,
            ..
        } = kind
        else {
            return Ok(None);
        };
        let ty::FnDef(def_id, generic_args) = *func.ty(&self.body.local_decls, self.tcx).kind()
        else {
            return Ok(None);
        };
        if self.tcx.def_path_str(def_id) != "std::iter::Iterator::next" {
            return Ok(None);
        }
        let ty::Adt(definition, _) = generic_args.type_at(0).kind() else {
            return Ok(None);
        };
        if !core_range_name(self.tcx, definition.did()).is_some_and(|name| name == "Range") {
            return Ok(None);
        }
        let Some(iterator) = args.first() else {
            return Err(self.block_error(block, "iterator `next` call has no iterator argument"));
        };
        let (MirOperand::Copy(place) | MirOperand::Move(place)) = &iterator.node else {
            return Err(self.block_error(block, "iterator `next` argument is not a local"));
        };
        if !place.projection.is_empty() {
            return Err(self.block_error(block, "iterator `next` argument has a projection"));
        }
        let LocalMapping::Scalar(iterator_local) = self.local_map[place.local.index()] else {
            return Err(self.block_error(block, "iterator `next` argument is not a scalar local"));
        };
        let OmType::Ref {
            kind: RefKind::Mutable,
            target: omlua_ir::RefTarget::Struct(_range_id),
        } = self.locals[iterator_local.index() as usize].ty
        else {
            return Err(self.block_error(block, "iterator `next` argument is not `&mut Range<i32>`"));
        };

        let destination = self.lower_place(block, destination)?;
        if !destination.path.is_empty() {
            return Err(self.block_error(block, "iterator `next` destination has a projection"));
        }
        let destination = destination.base;
        let target = target.ok_or_else(|| self.block_error(block, "diverging calls are not supported"))?;
        let target = block_id(target, &self.name)?;
        let OmType::Enum(option_id) = self.locals[destination.index() as usize].ty else {
            return Err(self.block_error(block, "iterator `next` destination is not an option"));
        };

        let start = push_local(&mut self.locals, OmType::I32, LocalKind::Temporary, &self.name)?;
        let end = push_local(&mut self.locals, OmType::I32, LocalKind::Temporary, &self.name)?;
        let has_next = push_local(&mut self.locals, OmType::Bool, LocalKind::Temporary, &self.name)?;
        let advanced = push_local(&mut self.locals, OmType::I32, LocalKind::Temporary, &self.name)?;
        let start_path = vec![ProjectElem::Deref, ProjectElem::Field(FieldId::new(0))];
        let end_path = vec![ProjectElem::Deref, ProjectElem::Field(FieldId::new(1))];

        let statements = vec![
            Statement::Assign {
                destination: start,
                value: Rvalue::Use(Operand::Project {
                    base: iterator_local,
                    path: start_path.clone(),
                    moved: false,
                }),
            },
            Statement::Assign {
                destination: end,
                value: Rvalue::Use(Operand::Project {
                    base: iterator_local,
                    path: end_path,
                    moved: false,
                }),
            },
            Statement::Assign {
                destination: has_next,
                value: Rvalue::Binary {
                    op: BinaryOp::Lt,
                    left: Operand::Copy(start),
                    right: Operand::Copy(end),
                },
            },
        ];

        let none_block = self.push_synthetic_block(
            block,
            vec![Statement::Assign {
                destination,
                value: Rvalue::Variant {
                    ty: option_id,
                    variant: VariantId::new(0),
                    fields: vec![],
                },
            }],
            Terminator::Goto { target },
        )?;
        let some_block = self.push_synthetic_block(
            block,
            vec![
                Statement::Assign {
                    destination: advanced,
                    value: Rvalue::Binary {
                        op: BinaryOp::Add,
                        left: Operand::Copy(start),
                        right: Operand::Constant(Constant::I32(1)),
                    },
                },
                Statement::Store {
                    destination: OmPlace {
                        base: iterator_local,
                        path: start_path,
                    },
                    value: Rvalue::Use(Operand::Copy(advanced)),
                },
                Statement::Assign {
                    destination,
                    value: Rvalue::Variant {
                        ty: option_id,
                        variant: VariantId::new(1),
                        fields: vec![Operand::Move(start)],
                    },
                },
            ],
            Terminator::Goto { target },
        )?;
        let unreachable_block =
            self.push_synthetic_block(block, Vec::new(), Terminator::Unreachable)?;

        let terminator = Terminator::SwitchInt {
            discriminant: Operand::Move(has_next),
            targets: vec![
                (SwitchValue(0), none_block),
                (SwitchValue(1), some_block),
            ],
            otherwise: unreachable_block,
        };
        Ok(Some((statements, terminator)))
    }

    fn push_synthetic_block(
        &mut self,
        source: BasicBlock,
        statements: Vec<Statement>,
        terminator: Terminator,
    ) -> Result<BlockId, LowerError> {
        let id = BlockId::new(self.next_synthetic_block);
        self.next_synthetic_block = self
            .next_synthetic_block
            .checked_add(1)
            .ok_or_else(|| self.block_error(source, "synthetic block ID overflow"))?;
        self.synthetic_blocks.push(OmBlock {
            id,
            statements,
            terminator,
        });
        Ok(id)
    }

    fn lower_terminator(
        &mut self,
        block: BasicBlock,
        kind: &TerminatorKind<'tcx>,
    ) -> Result<Terminator, LowerError> {
        match kind {
            TerminatorKind::Goto { target } => Ok(Terminator::Goto {
                target: block_id(*target, &self.name)?,
            }),
            TerminatorKind::SwitchInt { discr, targets } => Ok(Terminator::SwitchInt {
                discriminant: self.lower_operand(block, discr)?,
                targets: targets
                    .iter()
                    .map(|(value, target)| Ok((SwitchValue(value), block_id(target, &self.name)?)))
                    .collect::<Result<_, LowerError>>()?,
                otherwise: block_id(targets.otherwise(), &self.name)?,
            }),
            TerminatorKind::Call {
                func,
                args,
                destination,
                target,
                unwind,
                ..
            } => {
                let ty::FnDef(def_id, generic_args) =
                    *func.ty(&self.body.local_decls, self.tcx).kind()
                else {
                    return Err(self.block_error(block, "indirect calls are not supported"));
                };
                let destination_ty = destination.ty(&self.body.local_decls, self.tcx).ty;
                let target = target
                    .ok_or_else(|| self.block_error(block, "diverging calls are not supported"))?;
                if let Some(synthetic_call) =
                    self.classify_synthetic_call(block, def_id, generic_args, destination_ty)?
                {
                    let arguments = match synthetic_call {
                        SyntheticCall::OptionFromResidual { .. } => Vec::new(),
                        _ => args
                            .iter()
                            .map(|argument| self.lower_operand(block, &argument.node))
                            .collect::<Result<_, _>>()?,
                    };
                    let callee = self
                        .registry
                        .register_synthetic(self.tcx, synthetic_call.name(), &synthetic_call)
                        .map_err(|error| self.block_error(block, error.detail()))?;
                    return Ok(Terminator::Call {
                        callee,
                        arguments,
                        destination: self.lower_place(block, destination)?,
                        target: block_id(target, &self.name)?,
                        unwind: lower_unwind(*unwind, &self.name)?,
                    });
                }
                let callee = self
                    .registry
                    .register(self.tcx, def_id, generic_args)
                    .map_err(|error| self.block_error(block, error.detail()))?;
                Ok(Terminator::Call {
                    callee,
                    arguments: args
                        .iter()
                        .map(|argument| self.lower_operand(block, &argument.node))
                        .collect::<Result<_, _>>()?,
                    destination: self.lower_place(block, destination)?,
                    target: block_id(target, &self.name)?,
                    unwind: lower_unwind(*unwind, &self.name)?,
                })
            }
            TerminatorKind::Assert {
                cond,
                expected,
                msg,
                target,
                unwind,
            } => Ok(Terminator::Assert {
                condition: self.lower_operand(block, cond)?,
                expected: *expected,
                kind: self.lower_assert_kind(block, msg)?,
                target: block_id(*target, &self.name)?,
                unwind: lower_unwind(*unwind, &self.name)?,
            }),
            TerminatorKind::Return => Ok(Terminator::Return),
            TerminatorKind::Unreachable => Ok(Terminator::Unreachable),
            other => Err(self.block_error(
                block,
                format!("MIR terminator `{other:?}` is not supported"),
            )),
        }
    }

    fn lower_assert_kind(
        &self,
        block: BasicBlock,
        kind: &MirAssertKind<MirOperand<'tcx>>,
    ) -> Result<AssertKind, LowerError> {
        match kind {
            MirAssertKind::Overflow(op, left, right) => Ok(AssertKind::Overflow {
                op: lower_overflow_assert(*op).ok_or_else(|| {
                    self.block_error(
                        block,
                        format!("overflow assertion `{op:?}` is not supported"),
                    )
                })?,
                left: self.lower_operand(block, left)?,
                right: self.lower_operand(block, right)?,
            }),
            MirAssertKind::OverflowNeg(operand) => {
                Ok(AssertKind::OverflowNeg(self.lower_operand(block, operand)?))
            }
            MirAssertKind::DivisionByZero(operand) => Ok(AssertKind::DivisionByZero(
                self.lower_operand(block, operand)?,
            )),
            MirAssertKind::RemainderByZero(operand) => Ok(AssertKind::RemainderByZero(
                self.lower_operand(block, operand)?,
            )),
            other => {
                Err(self.block_error(block, format!("assertion `{other:?}` is not supported")))
            }
        }
    }

    fn block_error(&self, block: BasicBlock, detail: impl Into<String>) -> LowerError {
        LowerError::block(&self.name, block.index() as u32, detail)
    }

    fn classify_synthetic_call(
        &self,
        block: BasicBlock,
        def_id: DefId,
        generic_args: ty::GenericArgsRef<'tcx>,
        flow: Ty<'tcx>,
    ) -> Result<Option<SyntheticCall<'tcx>>, LowerError> {
        match self.tcx.def_path_str(def_id).as_str() {
            "std::ops::Try::branch" => self
                .try_branch_call(block, generic_args.type_at(0), flow)
                .map(Some),
            "std::ops::FromResidual::from_residual" => self
                .try_from_residual_call(block, generic_args.type_at(0), generic_args.type_at(1))
                .map(Some),
            "std::iter::IntoIterator::into_iter" => self
                .range_into_iter_call(block, generic_args.type_at(0))
                .map(Some),
            _ => Ok(None),
        }
    }

    fn try_branch_call(
        &self,
        block: BasicBlock,
        self_ty: Ty<'tcx>,
        flow: Ty<'tcx>,
    ) -> Result<SyntheticCall<'tcx>, LowerError> {
        let ty::Adt(definition, _) = self_ty.kind() else {
            return Err(self.unsupported_try(block, self_ty));
        };
        match core_try_enum_name(self.tcx, definition.did()) {
            Some("Option") => Ok(SyntheticCall::OptionBranch {
                option: self_ty,
                flow,
            }),
            Some("Result") => Ok(SyntheticCall::ResultBranch {
                result: self_ty,
                flow,
            }),
            _ => Err(self.unsupported_try(block, self_ty)),
        }
    }

    fn try_from_residual_call(
        &self,
        block: BasicBlock,
        self_ty: Ty<'tcx>,
        residual: Ty<'tcx>,
    ) -> Result<SyntheticCall<'tcx>, LowerError> {
        let ty::Adt(definition, _) = self_ty.kind() else {
            return Err(self.unsupported_try(block, self_ty));
        };
        match core_try_enum_name(self.tcx, definition.did()) {
            Some("Option") => Ok(SyntheticCall::OptionFromResidual { option: self_ty }),
            Some("Result") => {
                let ty::Adt(_, result_arguments) = self_ty.kind() else {
                    unreachable!("Result was classified as an ADT above");
                };
                let ty::Adt(residual_definition, residual_arguments) = residual.kind() else {
                    return Err(self.block_error(
                        block,
                        format!("unexpected Result residual type `{residual}`"),
                    ));
                };
                if !core_try_enum_name(self.tcx, residual_definition.did())
                    .is_some_and(|name| name == "Result")
                {
                    return Err(self.block_error(
                        block,
                        format!("unexpected Result residual type `{residual}`"),
                    ));
                }

                let target_error = result_arguments.type_at(1);
                let residual_error = residual_arguments.type_at(1);
                if target_error != residual_error {
                    return Err(self.block_error(
                        block,
                        format!(
                            "the `?` operator requires converting Result error `{residual_error}` into `{target_error}`; error conversion is not supported yet"
                        ),
                    ));
                }

                Ok(SyntheticCall::ResultFromResidual {
                    result: self_ty,
                    residual,
                })
            }
            _ => Err(self.unsupported_try(block, self_ty)),
        }
    }

    fn range_into_iter_call(
        &self,
        block: BasicBlock,
        self_ty: Ty<'tcx>,
    ) -> Result<SyntheticCall<'tcx>, LowerError> {
        let ty::Adt(definition, _) = self_ty.kind() else {
            return Err(self.unsupported_for(block, self_ty));
        };
        if !core_range_name(self.tcx, definition.did()).is_some_and(|name| name == "Range") {
            return Err(self.unsupported_for(block, self_ty));
        }
        Ok(SyntheticCall::RangeIntoIter { range: self_ty })
    }

    fn unsupported_try(&self, block: BasicBlock, self_ty: Ty<'tcx>) -> LowerError {
        self.block_error(
            block,
            format!(
                "the `?` operator is not supported for `{self_ty}`; only `Option` and `Result` are supported"
            ),
        )
    }

    fn unsupported_for(&self, block: BasicBlock, self_ty: Ty<'tcx>) -> LowerError {
        self.block_error(
            block,
            format!(
                "the `for` loop is not supported for `{self_ty}`; only integer ranges `0..N` are supported"
            ),
        )
    }

    fn struct_field(
        &self,
        block: BasicBlock,
        type_id: TypeId,
        field_index: usize,
    ) -> Result<&omlua_ir::OmField, LowerError> {
        self.registry
            .types
            .definition(type_id)
            .ok_or_else(|| {
                self.block_error(block, format!("structure type @{type_id} is incomplete"))
            })?
            .fields
            .get(field_index)
            .filter(|field| field.id.index() == field_index as u32)
            .ok_or_else(|| {
                self.block_error(
                    block,
                    format!("field .{field_index} does not exist in structure @{type_id}"),
                )
            })
    }

    fn validate_variant(
        &self,
        block: BasicBlock,
        type_id: TypeId,
        variant: VariantId,
    ) -> Result<(), LowerError> {
        let definition = self
            .registry
            .types
            .enum_definition(type_id)
            .ok_or_else(|| {
                self.block_error(block, format!("enum type @{type_id} is incomplete"))
            })?;
        if definition
            .variants
            .get(variant.index() as usize)
            .is_some_and(|v| v.id == variant)
        {
            Ok(())
        } else {
            Err(self.block_error(
                block,
                format!("variant v{variant} does not exist in enum @{type_id}"),
            ))
        }
    }

    fn variant_field(
        &self,
        block: BasicBlock,
        type_id: TypeId,
        variant: VariantId,
        field_index: usize,
    ) -> Result<&omlua_ir::OmField, LowerError> {
        let definition = self
            .registry
            .types
            .enum_definition(type_id)
            .ok_or_else(|| {
                self.block_error(block, format!("enum type @{type_id} is incomplete"))
            })?;
        definition
            .variants
            .get(variant.index() as usize)
            .filter(|v| v.id == variant)
            .ok_or_else(|| {
                self.block_error(
                    block,
                    format!("variant v{variant} does not exist in enum @{type_id}"),
                )
            })?
            .fields
            .get(field_index)
            .filter(|field| field.id.index() == field_index as u32)
            .ok_or_else(|| {
                self.block_error(
                    block,
                    format!(
                        "field .{field_index} does not exist in variant v{variant} of enum @{type_id}"
                    ),
                )
            })
    }
}

fn find_checked_locals(body: &Body<'_>) -> HashSet<Local> {
    body.basic_blocks
        .iter()
        .flat_map(|block| &block.statements)
        .filter_map(|statement| {
            let StatementKind::Assign(assignment) = &statement.kind else {
                return None;
            };
            let (place, value) = &**assignment;
            (place.projection.is_empty() && checked_binary(value).is_some()).then_some(place.local)
        })
        .collect()
}

fn find_discriminant_locals(body: &Body<'_>) -> HashSet<Local> {
    body.basic_blocks
        .iter()
        .flat_map(|block| &block.statements)
        .filter_map(|statement| {
            let StatementKind::Assign(assignment) = &statement.kind else {
                return None;
            };
            let (place, value) = &**assignment;
            (place.projection.is_empty() && matches!(value, MirRvalue::Discriminant(_)))
                .then_some(place.local)
        })
        .collect()
}

fn validate_checked_pair_type(ty: Ty<'_>) -> Result<(), &'static str> {
    let ty::Tuple(fields) = ty.kind() else {
        return Err("checked arithmetic result is not a tuple");
    };
    if fields.len() != 2
        || !matches!(fields[0].kind(), ty::Int(IntTy::I32))
        || !matches!(fields[1].kind(), ty::Bool)
    {
        return Err("checked arithmetic tuple is not `(i32, bool)`");
    }
    Ok(())
}

fn push_local(
    locals: &mut Vec<OmLocal>,
    ty: OmType,
    kind: LocalKind,
    function: &str,
) -> Result<LocalId, LowerError> {
    let index = u32::try_from(locals.len())
        .map_err(|_| LowerError::function(function, "local count exceeds OMIR limits"))?;
    let id = LocalId::new(index);
    locals.push(OmLocal { id, ty, kind });
    Ok(id)
}

fn block_id(block: BasicBlock, function: &str) -> Result<BlockId, LowerError> {
    let index = u32::try_from(block.index())
        .map_err(|_| LowerError::function(function, "basic block count exceeds OMIR limits"))?;
    Ok(BlockId::new(index))
}

fn checked_binary(value: &MirRvalue<'_>) -> Option<CheckedBinaryOp> {
    let MirRvalue::BinaryOp(op, _) = value else {
        return None;
    };
    match op {
        MirBinOp::AddWithOverflow => Some(CheckedBinaryOp::Add),
        MirBinOp::SubWithOverflow => Some(CheckedBinaryOp::Sub),
        MirBinOp::MulWithOverflow => Some(CheckedBinaryOp::Mul),
        _ => None,
    }
}

fn lower_unary(op: MirUnOp, operand: Ty<'_>) -> Option<UnaryOp> {
    match op {
        MirUnOp::Neg if matches!(operand.kind(), ty::Int(IntTy::I32)) => Some(UnaryOp::Neg),
        MirUnOp::Not if matches!(operand.kind(), ty::Bool) => Some(UnaryOp::Not),
        MirUnOp::Not if matches!(operand.kind(), ty::Int(IntTy::I32)) => Some(UnaryOp::BitNot),
        _ => None,
    }
}

fn lower_binary(op: MirBinOp, left: Ty<'_>, right: Ty<'_>) -> Option<BinaryOp> {
    let both_bool = matches!(left.kind(), ty::Bool) && matches!(right.kind(), ty::Bool);
    let both_i32 = matches!(left.kind(), ty::Int(IntTy::I32))
        && matches!(right.kind(), ty::Int(IntTy::I32));

    match op {
        MirBinOp::BitAnd if both_bool => Some(BinaryOp::And),
        MirBinOp::Add if both_i32 => Some(BinaryOp::Add),
        MirBinOp::Sub if both_i32 => Some(BinaryOp::Sub),
        MirBinOp::Mul if both_i32 => Some(BinaryOp::Mul),
        MirBinOp::Div if both_i32 => Some(BinaryOp::Div),
        MirBinOp::Rem if both_i32 => Some(BinaryOp::Rem),
        MirBinOp::Eq if both_bool || both_i32 => Some(BinaryOp::Eq),
        MirBinOp::Ne if both_bool || both_i32 => Some(BinaryOp::Ne),
        MirBinOp::Lt if both_i32 || both_bool => Some(BinaryOp::Lt),
        MirBinOp::Le if both_i32 || both_bool => Some(BinaryOp::Le),
        MirBinOp::Gt if both_i32 || both_bool => Some(BinaryOp::Gt),
        MirBinOp::Ge if both_i32 || both_bool => Some(BinaryOp::Ge),
        _ => None,
    }
}

fn lower_overflow_assert(op: MirBinOp) -> Option<BinaryOp> {
    match op {
        MirBinOp::Add => Some(BinaryOp::Add),
        MirBinOp::Sub => Some(BinaryOp::Sub),
        MirBinOp::Mul => Some(BinaryOp::Mul),
        MirBinOp::Div => Some(BinaryOp::Div),
        MirBinOp::Rem => Some(BinaryOp::Rem),
        _ => None,
    }
}

fn lower_unwind(action: MirUnwindAction, function: &str) -> Result<UnwindAction, LowerError> {
    match action {
        MirUnwindAction::Continue => Ok(UnwindAction::Continue),
        MirUnwindAction::Unreachable => Ok(UnwindAction::Unreachable),
        MirUnwindAction::Terminate(UnwindTerminateReason::Abi) => Ok(UnwindAction::TerminateAbi),
        MirUnwindAction::Terminate(UnwindTerminateReason::InCleanup) => {
            Ok(UnwindAction::TerminateInCleanup)
        }
        MirUnwindAction::Cleanup(block) => Ok(UnwindAction::Cleanup(block_id(block, function)?)),
    }
}
