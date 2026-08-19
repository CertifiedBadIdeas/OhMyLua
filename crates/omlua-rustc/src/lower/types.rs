use std::collections::HashMap;

use omlua_ir::{FieldId, OmEnum, OmField, OmStruct, OmType, OmVariant, TypeId, VariantId};
use rustc_hir::Mutability;
use rustc_middle::ty::{self, IntTy, Ty, TyCtxt};
use rustc_span::def_id::DefId;

use crate::LowerError;

pub(super) struct TypeRegistry {
    ids: HashMap<DefId, TypeId>,
    definitions: Vec<Option<OmStruct>>,
    enum_definitions: Vec<Option<OmEnum>>,
}

impl TypeRegistry {
    pub(super) fn new() -> Self {
        Self {
            ids: HashMap::new(),
            definitions: Vec::new(),
            enum_definitions: Vec::new(),
        }
    }

    pub(super) fn lower_type<'tcx>(
        &mut self,
        tcx: TyCtxt<'tcx>,
        ty: Ty<'tcx>,
    ) -> Result<OmType, LowerError> {
        match ty.kind() {
            ty::Tuple(fields) if fields.is_empty() => Ok(OmType::Unit),
            ty::Bool => Ok(OmType::Bool),
            ty::Int(IntTy::I32) => Ok(OmType::I32),
            ty::Adt(definition, arguments) if definition.is_struct() => self
                .register_struct(tcx, definition.did(), arguments)
                .map(OmType::Struct),
            ty::Adt(definition, arguments) if definition.is_enum() => self
                .register_enum(tcx, definition.did(), arguments)
                .map(OmType::Enum),
            ty::Ref(_, inner, Mutability::Not) => {
                let ty::Adt(definition, arguments) = inner.kind() else {
                    return Err(LowerError::program(format!(
                        "shared reference `{ty}` is not supported; only references to named structures are supported"
                    )));
                };
                if !definition.is_struct() {
                    return Err(LowerError::program(format!(
                        "shared reference `{ty}` is not supported; only references to named structures are supported"
                    )));
                }
                self.register_struct(tcx, definition.did(), arguments)
                    .map(OmType::SharedRef)
            }
            ty::Ref(_, _, Mutability::Mut) => Err(LowerError::program(format!(
                "mutable reference `{ty}` is not supported"
            ))),
            _ => Err(LowerError::program(format!("type `{ty}` is not supported"))),
        }
    }

    pub(super) fn type_id(&self, def_id: DefId) -> Option<TypeId> {
        self.ids.get(&def_id).copied()
    }

    pub(super) fn definition(&self, id: TypeId) -> Option<&OmStruct> {
        self.definitions
            .get(id.index() as usize)
            .and_then(Option::as_ref)
    }

    pub(super) fn enum_definition(&self, id: TypeId) -> Option<&OmEnum> {
        self.enum_definitions
            .get(id.index() as usize)
            .and_then(Option::as_ref)
    }

    pub(super) fn finish(self) -> Result<(Vec<OmStruct>, Vec<OmEnum>), LowerError> {
        let structs = self
            .definitions
            .into_iter()
            .enumerate()
            .map(|(index, definition)| {
                definition.ok_or_else(|| {
                    LowerError::program(format!(
                        "structure type @{index} was not completely defined"
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let enums = self
            .enum_definitions
            .into_iter()
            .enumerate()
            .map(|(index, definition)| {
                definition.ok_or_else(|| {
                    LowerError::program(format!("enum type @{index} was not completely defined"))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok((structs, enums))
    }

    fn register_struct<'tcx>(
        &mut self,
        tcx: TyCtxt<'tcx>,
        def_id: DefId,
        arguments: ty::GenericArgsRef<'tcx>,
    ) -> Result<TypeId, LowerError> {
        if !def_id.is_local() {
            return Err(LowerError::program(format!(
                "external structure `{}` is not supported",
                tcx.def_path_str(def_id)
            )));
        }
        if !arguments.is_empty() || tcx.generics_of(def_id).count() != 0 {
            return Err(LowerError::program(format!(
                "generic structure `{}` is not supported",
                tcx.def_path_str(def_id)
            )));
        }
        if let Some(id) = self.ids.get(&def_id) {
            return Ok(*id);
        }

        let definition = tcx.adt_def(def_id);
        let variant = definition.non_enum_variant();
        if variant.ctor.is_some() {
            let kind = if variant.fields.is_empty() {
                "unit"
            } else {
                "tuple"
            };
            return Err(LowerError::program(format!(
                "{kind} struct `{}` is not supported",
                tcx.item_name(def_id)
            )));
        }
        if definition.has_dtor(tcx) {
            return Err(LowerError::program(format!(
                "structure `{}` with a destructor is not supported",
                tcx.def_path_str(def_id)
            )));
        }

        let index = u32::try_from(self.definitions.len())
            .map_err(|_| LowerError::program("structure count exceeds OMIR limits"))?;
        let id = TypeId::new(index);
        self.ids.insert(def_id, id);
        self.definitions.push(None);

        let mut fields = Vec::with_capacity(variant.fields.len());
        for (index, field) in variant.fields.iter().enumerate() {
            let field_id = FieldId::new(
                u32::try_from(index)
                    .map_err(|_| LowerError::program("field count exceeds OMIR limits"))?,
            );
            let field_ty = field.ty(tcx, arguments).skip_normalization();
            if matches!(field_ty.kind(), ty::Ref(..)) {
                return Err(LowerError::program(format!(
                    "reference field `{}` in structure `{}` is not supported",
                    field.name,
                    tcx.def_path_str(def_id)
                )));
            }
            let ty = self.lower_type(tcx, field_ty)?;
            if ty == OmType::Unit {
                return Err(LowerError::program(format!(
                    "unit field `{}` in structure `{}` is not supported",
                    field.name,
                    tcx.def_path_str(def_id)
                )));
            }
            fields.push(OmField {
                id: field_id,
                name: field.name.as_str().to_owned(),
                ty,
            });
        }

        self.definitions[id.index() as usize] = Some(OmStruct {
            id,
            name: tcx.def_path_str(def_id),
            fields,
        });
        Ok(id)
    }

    fn register_enum<'tcx>(
        &mut self,
        tcx: TyCtxt<'tcx>,
        def_id: DefId,
        arguments: ty::GenericArgsRef<'tcx>,
    ) -> Result<TypeId, LowerError> {
        if !def_id.is_local() {
            return Err(LowerError::program(format!(
                "external enum `{}` is not supported",
                tcx.def_path_str(def_id)
            )));
        }
        if !arguments.is_empty() || tcx.generics_of(def_id).count() != 0 {
            return Err(LowerError::program(format!(
                "generic enum `{}` is not supported",
                tcx.def_path_str(def_id)
            )));
        }
        if let Some(id) = self.ids.get(&def_id) {
            return Ok(*id);
        }

        let definition = tcx.adt_def(def_id);
        if definition.has_dtor(tcx) {
            return Err(LowerError::program(format!(
                "enum `{}` with a destructor is not supported",
                tcx.def_path_str(def_id)
            )));
        }

        let index = u32::try_from(self.enum_definitions.len())
            .map_err(|_| LowerError::program("enum count exceeds OMIR limits"))?;
        let id = TypeId::new(index);
        self.ids.insert(def_id, id);
        self.enum_definitions.push(None);

        let mut variants = Vec::with_capacity(definition.variants().len());
        for (variant_index, variant) in definition.variants().iter_enumerated() {
            let variant_id = VariantId::new(
                u32::try_from(variant_index)
                    .map_err(|_| LowerError::program("variant count exceeds OMIR limits"))?,
            );
            let mut fields = Vec::with_capacity(variant.fields.len());
            for (field_index, field) in variant.fields.iter().enumerate() {
                let field_id = FieldId::new(
                    u32::try_from(field_index)
                        .map_err(|_| LowerError::program("field count exceeds OMIR limits"))?,
                );
                let field_ty = field.ty(tcx, arguments).skip_normalization();
                if matches!(field_ty.kind(), ty::Ref(..)) {
                    return Err(LowerError::program(format!(
                        "reference field `{}` in variant `{}` of enum `{}` is not supported",
                        field.name,
                        variant.name,
                        tcx.def_path_str(def_id)
                    )));
                }
                let ty = self.lower_type(tcx, field_ty)?;
                if ty == OmType::Unit {
                    return Err(LowerError::program(format!(
                        "unit field `{}` in variant `{}` of enum `{}` is not supported",
                        field.name,
                        variant.name,
                        tcx.def_path_str(def_id)
                    )));
                }
                fields.push(OmField {
                    id: field_id,
                    name: field.name.as_str().to_owned(),
                    ty,
                });
            }
            variants.push(OmVariant {
                id: variant_id,
                name: variant.name.as_str().to_owned(),
                fields,
            });
        }

        self.enum_definitions[id.index() as usize] = Some(OmEnum {
            id,
            name: tcx.def_path_str(def_id),
            variants,
        });
        Ok(id)
    }
}
