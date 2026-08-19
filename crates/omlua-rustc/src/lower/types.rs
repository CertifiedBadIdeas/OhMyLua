use std::collections::HashMap;

use omlua_ir::{FieldId, OmEnum, OmField, OmStruct, OmType, OmVariant, TypeId, VariantId};
use rustc_hir::Mutability;
use rustc_middle::ty::{self, IntTy, Ty, TyCtxt};
use rustc_span::def_id::DefId;

use crate::LowerError;

pub(super) struct TypeRegistry {
    struct_ids: HashMap<DefId, TypeId>,
    local_enum_ids: HashMap<DefId, TypeId>,
    core_enum_ids: HashMap<String, TypeId>,
    definitions: Vec<Option<Nominal>>,
}

enum Nominal {
    Struct(OmStruct),
    Enum(OmEnum),
}

impl TypeRegistry {
    pub(super) fn new() -> Self {
        Self {
            struct_ids: HashMap::new(),
            local_enum_ids: HashMap::new(),
            core_enum_ids: HashMap::new(),
            definitions: Vec::new(),
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

    pub(super) fn definition(&self, id: TypeId) -> Option<&OmStruct> {
        match self.definitions.get(id.index() as usize) {
            Some(Some(Nominal::Struct(definition))) => Some(definition),
            _ => None,
        }
    }

    pub(super) fn enum_definition(&self, id: TypeId) -> Option<&OmEnum> {
        match self.definitions.get(id.index() as usize) {
            Some(Some(Nominal::Enum(definition))) => Some(definition),
            _ => None,
        }
    }

    pub(super) fn finish(self) -> Result<(Vec<OmStruct>, Vec<OmEnum>), LowerError> {
        let mut structs = Vec::new();
        let mut enums = Vec::new();
        for (index, definition) in self.definitions.into_iter().enumerate() {
            match definition.ok_or_else(|| {
                LowerError::program(format!("type @{index} was not completely defined"))
            })? {
                Nominal::Struct(definition) => structs.push(definition),
                Nominal::Enum(definition) => enums.push(definition),
            }
        }
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
        if let Some(id) = self.struct_ids.get(&def_id) {
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
        self.struct_ids.insert(def_id, id);
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

        self.definitions[id.index() as usize] = Some(Nominal::Struct(OmStruct {
            id,
            name: tcx.def_path_str(def_id),
            fields,
        }));
        Ok(id)
    }

    fn register_enum<'tcx>(
        &mut self,
        tcx: TyCtxt<'tcx>,
        def_id: DefId,
        arguments: ty::GenericArgsRef<'tcx>,
    ) -> Result<TypeId, LowerError> {
        let name = if def_id.is_local() {
            if !arguments.is_empty() || tcx.generics_of(def_id).count() != 0 {
                return Err(LowerError::program(format!(
                    "generic enum `{}` is not supported",
                    tcx.def_path_str(def_id)
                )));
            }
            if let Some(id) = self.local_enum_ids.get(&def_id) {
                return Ok(*id);
            }
            tcx.def_path_str(def_id)
        } else {
            let base = core_try_enum_name(tcx, def_id).ok_or_else(|| {
                LowerError::program(format!(
                    "external enum `{}` is not supported",
                    tcx.def_path_str(def_id)
                ))
            })?;
            let name = core_enum_name(base, arguments);
            if let Some(id) = self.core_enum_ids.get(&name) {
                return Ok(*id);
            }
            name
        };

        let definition = tcx.adt_def(def_id);
        if definition.has_dtor(tcx) {
            return Err(LowerError::program(format!(
                "enum `{}` with a destructor is not supported",
                tcx.def_path_str(def_id)
            )));
        }

        let index = u32::try_from(self.definitions.len())
            .map_err(|_| LowerError::program("enum count exceeds OMIR limits"))?;
        let id = TypeId::new(index);
        if def_id.is_local() {
            self.local_enum_ids.insert(def_id, id);
        } else {
            self.core_enum_ids.insert(name.clone(), id);
        }
        self.definitions.push(None);

        let mut variants = Vec::with_capacity(definition.variants().len());
        for (variant_index, variant) in definition.variants().iter_enumerated() {
            let variant_id = VariantId::new(variant_index.as_u32());
            let mut fields = Vec::with_capacity(variant.fields.len());
            for (field_index, field) in variant.fields.iter_enumerated() {
                let field_id = FieldId::new(field_index.as_u32());
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

        self.definitions[id.index() as usize] = Some(Nominal::Enum(OmEnum {
            id,
            name,
            variants,
        }));
        Ok(id)
    }
}

fn core_try_enum_name(tcx: TyCtxt<'_>, def_id: DefId) -> Option<&'static str> {
    match tcx.def_path_str(def_id).as_str() {
        "std::option::Option" => Some("Option"),
        "std::result::Result" => Some("Result"),
        "std::ops::ControlFlow" => Some("ControlFlow"),
        "std::convert::Infallible" => Some("Infallible"),
        _ => None,
    }
}

fn core_enum_name(base: &'static str, arguments: ty::GenericArgsRef<'_>) -> String {
    if arguments.is_empty() {
        return base.to_owned();
    }
    format!(
        "{base}<{}>",
        arguments
            .types()
            .map(|ty| ty.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    )
    .replace("std::result::Result", "Result")
    .replace("std::option::Option", "Option")
    .replace("std::ops::ControlFlow", "ControlFlow")
    .replace("std::convert::Infallible", "Infallible")
}
