use std::collections::HashSet;

use omlua_ir::{
    AssertKind, BinaryOp, BlockId, CheckedBinaryOp, Constant, FieldId, FunctionId, LocalId,
    LocalKind, OmBlock, OmFunction, OmLocal, OmType, Operand, ProjectElem, Rvalue, Statement,
    SwitchValue, Terminator, TypeId, UnaryOp, UnwindAction, VariantId,
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

        Ok(Self {
            tcx,
            body,
            name,
            registry,
            locals,
            local_map,
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
        Ok(OmBlock {
            id: block_id(block, &self.name)?,
            statements,
            terminator: self.lower_terminator(block, &data.terminator().kind)?,
        })
    }

    fn lower_statement(
        &self,
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

                Ok(Some(Statement::Assign {
                    destination: self.lower_place(block, place)?,
                    value: self.lower_rvalue(block, value)?,
                }))
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
        &self,
        block: BasicBlock,
        value: &MirRvalue<'tcx>,
    ) -> Result<Rvalue, LowerError> {
        match value {
            MirRvalue::Use(operand, _) => Ok(Rvalue::Use(self.lower_operand(block, operand)?)),
            MirRvalue::Discriminant(place) => Ok(Rvalue::Discriminant {
                source: self.lower_operand_place(block, place, false)?,
            }),
            MirRvalue::UnaryOp(op, operand) => Ok(Rvalue::Unary {
                op: lower_unary(*op).ok_or_else(|| {
                    self.block_error(block, format!("unary operation `{op:?}` is not supported"))
                })?,
                operand: self.lower_operand(block, operand)?,
            }),
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
                let AggregateKind::Adt(def_id, variant, _, _, _) = kind.as_ref() else {
                    return Err(self.block_error(block, "non-structure aggregate is not supported"));
                };
                let definition = self.tcx.adt_def(*def_id);
                let ty = self.registry.types.type_id(*def_id).ok_or_else(|| {
                    self.block_error(
                        block,
                        format!(
                            "structure `{}` was not registered",
                            self.tcx.def_path_str(*def_id)
                        ),
                    )
                })?;
                let fields = operands
                    .iter()
                    .map(|operand| self.lower_operand(block, operand))
                    .collect::<Result<_, _>>()?;
                if definition.is_enum() {
                    Ok(Rvalue::Variant {
                        ty,
                        variant: VariantId::new(variant.as_u32()),
                        fields,
                    })
                } else {
                    if variant.as_usize() != 0 {
                        return Err(self.block_error(block, "non-structure aggregate is not supported"));
                    }
                    Ok(Rvalue::Struct { ty, fields })
                }
            }
            MirRvalue::Ref(_, borrow_kind, place) => {
                if !matches!(borrow_kind, BorrowKind::Shared) {
                    return Err(
                        self.block_error(block, "only shared structure borrows are supported")
                    );
                }
                let source = self.lower_shared_borrow_source(block, place)?;
                Ok(Rvalue::SharedBorrow { source })
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
        if matches!(
            self.local_map[place.local.index()],
            LocalMapping::CheckedPair { .. }
        ) {
            let local = self.lower_place(block, place)?;
            return Ok(if moved {
                Operand::Move(local)
            } else {
                Operand::Copy(local)
            });
        }

        let LocalMapping::Scalar(base) = self.local_map[place.local.index()] else {
            unreachable!()
        };
        if place.projection.is_empty() {
            return Ok(if moved {
                Operand::Move(base)
            } else {
                Operand::Copy(base)
            });
        }

        let mut ty = self.locals[base.index() as usize].ty;
        let mut path = Vec::new();
        let mut downcast: Option<VariantId> = None;
        for projection in place.projection.as_ref() {
            match projection {
                PlaceElem::Deref => {
                    let OmType::SharedRef(id) = ty else {
                        return Err(self.block_error(
                            block,
                            "dereference of a non-shared-structure reference is not supported",
                        ));
                    };
                    ty = OmType::Struct(id);
                    path.push(ProjectElem::Deref);
                }
                PlaceElem::Downcast(_name, variant) => {
                    if downcast.is_some() {
                        return Err(self.block_error(block, "nested downcast without a field read"));
                    }
                    let OmType::Enum(type_id) = ty else {
                        return Err(self.block_error(block, "downcast of a non-enum value"));
                    };
                    self.validate_variant(block, type_id, VariantId::new(variant.as_u32()))?;
                    downcast = Some(VariantId::new(variant.as_u32()));
                    path.push(ProjectElem::Downcast(VariantId::new(variant.as_u32())));
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
                            return Err(self.block_error(block, "field access on a non-structure value"));
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
            return Err(self.block_error(block, "enum downcast without a field read"));
        }
        Ok(Operand::Project { base, path, moved })
    }

    fn lower_shared_borrow_source(
        &self,
        block: BasicBlock,
        place: &Place<'tcx>,
    ) -> Result<Operand, LowerError> {
        let source = self.lower_operand_place(block, place, false)?;
        if !matches!(self.om_operand_type(&source)?, OmType::Struct(_)) {
            return Err(self.block_error(block, "shared borrow source is not a structure"));
        }
        Ok(source)
    }

    fn om_operand_type(&self, operand: &Operand) -> Result<OmType, LowerError> {
        match operand {
            Operand::Copy(local) | Operand::Move(local) => {
                Ok(self.locals[local.index() as usize].ty)
            }
            Operand::Project { base, path, .. } => {
                let mut ty = self.locals[base.index() as usize].ty;
                let mut downcast: Option<VariantId> = None;
                for element in path {
                    match element {
                        ProjectElem::Deref => {
                            let OmType::SharedRef(id) = ty else {
                                return Err(LowerError::function(
                                    &self.name,
                                    "dereference source is not a shared structure reference",
                                ));
                            };
                            ty = OmType::Struct(id);
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
                                            format!(
                                                "field .{field} does not exist in structure @{type_id}"
                                            ),
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
                                        "field projection crosses a non-structure value",
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
                        "projection ends with a downcast without a field read",
                    ));
                }
                Ok(ty)
            }
            Operand::Constant(Constant::Unit) => Ok(OmType::Unit),
            Operand::Constant(Constant::Bool(_)) => Ok(OmType::Bool),
            Operand::Constant(Constant::I32(_)) => Ok(OmType::I32),
        }
    }

    fn lower_place(&self, block: BasicBlock, place: &Place<'tcx>) -> Result<LocalId, LowerError> {
        match (
            self.local_map[place.local.index()],
            place.projection.as_ref(),
        ) {
            (LocalMapping::Scalar(id), []) => Ok(id),
            (LocalMapping::CheckedPair { value, .. }, [PlaceElem::Field(field, _)])
                if field.index() == 0 =>
            {
                Ok(value)
            }
            (LocalMapping::CheckedPair { overflow, .. }, [PlaceElem::Field(field, _)])
                if field.index() == 1 =>
            {
                Ok(overflow)
            }
            _ => Err(self.block_error(
                block,
                format!(
                    "place projection on local _{} is not supported",
                    place.local.index()
                ),
            )),
        }
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
                let callee = self
                    .registry
                    .register(self.tcx, def_id, generic_args)
                    .map_err(|error| self.block_error(block, error.detail()))?;
                let target = target
                    .ok_or_else(|| self.block_error(block, "diverging calls are not supported"))?;
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
        let definition = self.registry.types.enum_definition(type_id).ok_or_else(|| {
            self.block_error(block, format!("enum type @{type_id} is incomplete"))
        })?;
        if definition.variants.get(variant.index() as usize).is_some_and(|v| v.id == variant) {
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
        let definition = self.registry.types.enum_definition(type_id).ok_or_else(|| {
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

fn lower_unary(op: MirUnOp) -> Option<UnaryOp> {
    match op {
        MirUnOp::Neg => Some(UnaryOp::Neg),
        MirUnOp::Not => Some(UnaryOp::Not),
        _ => None,
    }
}

fn lower_binary(op: MirBinOp, left: Ty<'_>, right: Ty<'_>) -> Option<BinaryOp> {
    match op {
        MirBinOp::BitAnd if matches!(left.kind(), ty::Bool) && matches!(right.kind(), ty::Bool) => {
            Some(BinaryOp::And)
        }
        MirBinOp::Add => Some(BinaryOp::Add),
        MirBinOp::Sub => Some(BinaryOp::Sub),
        MirBinOp::Mul => Some(BinaryOp::Mul),
        MirBinOp::Div => Some(BinaryOp::Div),
        MirBinOp::Rem => Some(BinaryOp::Rem),
        MirBinOp::Eq => Some(BinaryOp::Eq),
        MirBinOp::Ne => Some(BinaryOp::Ne),
        MirBinOp::Lt => Some(BinaryOp::Lt),
        MirBinOp::Le => Some(BinaryOp::Le),
        MirBinOp::Gt => Some(BinaryOp::Gt),
        MirBinOp::Ge => Some(BinaryOp::Ge),
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
