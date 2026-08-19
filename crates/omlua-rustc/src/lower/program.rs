use std::collections::{HashMap, VecDeque};

use omlua_ir::{FunctionId, OmProgram};
use rustc_hir::def::DefKind;
use rustc_middle::ty::{GenericArgsRef, TyCtxt};
use rustc_span::def_id::DefId;

use crate::LowerError;

use super::body::lower_function;
use super::types::TypeRegistry;

pub(crate) fn lower_program(tcx: TyCtxt<'_>) -> Result<OmProgram, LowerError> {
    let (entry_def, _) = tcx
        .entry_fn(())
        .ok_or_else(|| LowerError::program("crate has no `main` entry function"))?;

    let mut registry = FunctionRegistry::new();
    let entry = registry.register(tcx, entry_def, GenericArgsRef::default())?;
    let mut functions = Vec::new();

    while let Some(def_id) = registry.pending.pop_front() {
        let id = registry.ids[&def_id];
        functions.push(lower_function(tcx, def_id, id, &mut registry)?);
    }

    let structs = registry.types.finish()?;
    Ok(OmProgram {
        entry,
        structs,
        functions,
    })
}

pub(super) struct FunctionRegistry {
    ids: HashMap<DefId, FunctionId>,
    pending: VecDeque<DefId>,
    pub(super) types: TypeRegistry,
}

impl FunctionRegistry {
    fn new() -> Self {
        Self {
            ids: HashMap::new(),
            pending: VecDeque::new(),
            types: TypeRegistry::new(),
        }
    }

    pub(super) fn register(
        &mut self,
        tcx: TyCtxt<'_>,
        def_id: DefId,
        generic_args: GenericArgsRef<'_>,
    ) -> Result<FunctionId, LowerError> {
        if !def_id.is_local() {
            return Err(LowerError::program(format!(
                "external call `{}` is not supported",
                tcx.def_path_str(def_id)
            )));
        }
        match tcx.def_kind(def_id) {
            DefKind::Fn => {}
            DefKind::AssocFn
                if tcx.trait_of_assoc(def_id).is_none() && tcx.trait_item_of(def_id).is_none() => {}
            DefKind::AssocFn => {
                return Err(LowerError::program(format!(
                    "trait method `{}` is not supported",
                    tcx.def_path_str(def_id)
                )));
            }
            _ => {
                return Err(LowerError::program(format!(
                    "callable `{}` is not a function",
                    tcx.def_path_str(def_id)
                )));
            }
        }
        if !generic_args.is_empty() || tcx.generics_of(def_id).count() != 0 {
            return Err(LowerError::program(format!(
                "generic function `{}` is not supported",
                tcx.def_path_str(def_id)
            )));
        }

        if let Some(id) = self.ids.get(&def_id) {
            return Ok(*id);
        }

        let index = u32::try_from(self.ids.len())
            .map_err(|_| LowerError::program("function count exceeds OMIR limits"))?;
        let id = FunctionId::new(index);
        self.ids.insert(def_id, id);
        self.pending.push_back(def_id);
        Ok(id)
    }
}
