use std::collections::{BTreeMap, BTreeSet};

use omlua_lua_backend::{LuaBackendProfile, LuaDialect};
use omlua_lua_ir::{
    LirBinaryOp, LirBlockId, LirExpression, LirFunction, LirFunctionId, LirLocalId, LirProgram,
    LirStatement, LirTerminator, LirUnaryOp, LirValue, LirValueKind, RuntimeHelper,
};

use crate::CodegenError;

const LUA54_MAX_LOCALS_PER_FUNCTION: usize = 200;

pub(crate) fn validate(
    program: &LirProgram,
    profile: &LuaBackendProfile,
) -> Result<(), CodegenError> {
    if profile.dialect() != LuaDialect::Lua54 {
        return Err(error("the selected profile is not Lua 5.4"));
    }
    if program.requirements.minimum_integer_bits > profile.numeric().integer_bits {
        return Err(error(format!(
            "LIR requires {}-bit integers, but the selected profile provides {} bits",
            program.requirements.minimum_integer_bits,
            profile.numeric().integer_bits
        )));
    }
    if program.requirements.label_jumps && !profile.control_flow().label_jumps {
        return Err(error("LIR requires label jumps"));
    }
    if program.requirements.native_bitwise && !profile.operators().native_bitwise {
        return Err(error("LIR requires native bitwise operators"));
    }

    let helpers = unique_helpers(&program.helpers)?;
    if program.helpers.len() + program.functions.len() > LUA54_MAX_LOCALS_PER_FUNCTION {
        return Err(error(format!(
            "the generated chunk would declare more than {LUA54_MAX_LOCALS_PER_FUNCTION} local names"
        )));
    }
    if let Some(remainder_index) = program
        .helpers
        .iter()
        .position(|helper| *helper == RuntimeHelper::I32Rem)
        && !program.helpers[..remainder_index].contains(&RuntimeHelper::I32DivTrunc)
    {
        return Err(error(
            "the i32 division helper must be declared before the i32 remainder helper",
        ));
    }
    let functions = unique_functions(&program.functions)?;
    let entry = functions.get(&program.entry).ok_or_else(|| {
        error(format!(
            "entry function f{} does not exist",
            program.entry.index()
        ))
    })?;
    if !entry.parameters.is_empty() {
        return Err(error(format!(
            "entry function f{} must not have parameters",
            program.entry.index()
        )));
    }

    let mut used_helpers = BTreeSet::new();
    for function in &program.functions {
        validate_function(function, &functions, &helpers, &mut used_helpers)?;
    }
    if helpers != used_helpers {
        return Err(error(
            "the declared runtime helpers do not exactly match their uses",
        ));
    }
    if helpers.contains(&RuntimeHelper::I32Rem) && !helpers.contains(&RuntimeHelper::I32DivTrunc) {
        return Err(error(
            "the i32 remainder helper requires the i32 division helper",
        ));
    }
    Ok(())
}

fn unique_helpers(helpers: &[RuntimeHelper]) -> Result<BTreeSet<RuntimeHelper>, CodegenError> {
    let mut result = BTreeSet::new();
    for helper in helpers {
        if !result.insert(*helper) {
            return Err(error(format!(
                "runtime helper {helper:?} is declared twice"
            )));
        }
    }
    Ok(result)
}

fn unique_functions(
    functions: &[LirFunction],
) -> Result<BTreeMap<LirFunctionId, &LirFunction>, CodegenError> {
    let mut result = BTreeMap::new();
    for function in functions {
        if result.insert(function.id, function).is_some() {
            return Err(error(format!(
                "function f{} is defined twice",
                function.id.index()
            )));
        }
    }
    Ok(result)
}

fn validate_function(
    function: &LirFunction,
    functions: &BTreeMap<LirFunctionId, &LirFunction>,
    helpers: &BTreeSet<RuntimeHelper>,
    used_helpers: &mut BTreeSet<RuntimeHelper>,
) -> Result<(), CodegenError> {
    let context = format!("function f{}", function.id.index());
    if function.locals.len() > LUA54_MAX_LOCALS_PER_FUNCTION {
        return Err(error(format!(
            "{context} declares more than {LUA54_MAX_LOCALS_PER_FUNCTION} local names"
        )));
    }
    let mut locals = BTreeMap::new();
    for local in &function.locals {
        if locals.insert(local.id, local.kind.clone()).is_some() {
            return Err(error(format!(
                "{context} defines local v{} twice",
                local.id.index()
            )));
        }
    }

    let parameter_set: BTreeSet<_> = function.parameters.iter().copied().collect();
    if parameter_set.len() != function.parameters.len() {
        return Err(error(format!("{context} has a duplicate parameter")));
    }
    for parameter in &function.parameters {
        if !locals.contains_key(parameter) {
            return Err(error(format!(
                "{context} references missing parameter v{}",
                parameter.index()
            )));
        }
    }
    for local in &function.locals {
        if local.parameter != parameter_set.contains(&local.id) {
            return Err(error(format!(
                "{context} has inconsistent parameter metadata for v{}",
                local.id.index()
            )));
        }
    }
    let return_kind = function
        .return_local
        .map(|id| local_kind(&locals, id, &context))
        .transpose()?;

    let mut blocks = BTreeSet::new();
    for block in &function.blocks {
        if !blocks.insert(block.id) {
            return Err(error(format!(
                "{context} defines block bb{} twice",
                block.id.index()
            )));
        }
    }
    require_block(&blocks, function.entry, &context)?;

    for block in &function.blocks {
        let block_context = format!("{context}, block bb{}", block.id.index());
        for statement in &block.statements {
            match statement {
                LirStatement::Assign { destination, value } => {
                    let expected = local_kind(&locals, *destination, &block_context)?;
                    let actual =
                        expression_kind(value, &locals, helpers, used_helpers, &block_context)?;
                    require_kind(expected, actual, &block_context, "assignment")?;
                }
            }
        }
        validate_terminator(
            &block.terminator,
            return_kind.clone(),
            &locals,
            &blocks,
            functions,
            helpers,
            used_helpers,
            &block_context,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_terminator(
    terminator: &LirTerminator,
    return_kind: Option<LirValueKind>,
    locals: &BTreeMap<LirLocalId, LirValueKind>,
    blocks: &BTreeSet<LirBlockId>,
    functions: &BTreeMap<LirFunctionId, &LirFunction>,
    helpers: &BTreeSet<RuntimeHelper>,
    used_helpers: &mut BTreeSet<RuntimeHelper>,
    context: &str,
) -> Result<(), CodegenError> {
    let mut expression =
        |value: &LirExpression| expression_kind(value, locals, helpers, used_helpers, context);
    match terminator {
        LirTerminator::Jump { target } => require_block(blocks, *target, context),
        LirTerminator::Switch {
            discriminant,
            targets,
            otherwise,
        } => {
            require_kind(
                LirValueKind::Integer,
                expression(discriminant)?,
                context,
                "switch",
            )?;
            let mut values = BTreeSet::new();
            for (value, target) in targets {
                if !values.insert(*value) {
                    return Err(error(format!(
                        "{context} has duplicate switch value {value}"
                    )));
                }
                require_block(blocks, *target, context)?;
            }
            require_block(blocks, *otherwise, context)
        }
        LirTerminator::Call {
            callee,
            arguments,
            destination,
            target,
        } => {
            let called = functions.get(callee).ok_or_else(|| {
                error(format!(
                    "{context} calls missing function f{}",
                    callee.index()
                ))
            })?;
            if arguments.len() != called.parameters.len() {
                return Err(error(format!(
                    "{context} passes {} arguments to f{}, which expects {}",
                    arguments.len(),
                    callee.index(),
                    called.parameters.len()
                )));
            }
            for (argument, parameter) in arguments.iter().zip(&called.parameters) {
                let expected = called
                    .locals
                    .iter()
                    .find(|local| local.id == *parameter)
                    .map(|local| local.kind.clone())
                    .ok_or_else(|| {
                        error(format!(
                            "function f{} has an invalid parameter",
                            callee.index()
                        ))
                    })?;
                let actual = expression(argument)?;
                require_kind(expected, actual, context, "call argument")?;
            }
            let called_return = called
                .return_local
                .map(|id| {
                    called
                        .locals
                        .iter()
                        .find(|local| local.id == id)
                        .map(|local| local.kind.clone())
                        .ok_or_else(|| {
                            error(format!(
                                "function f{} has an invalid return local",
                                callee.index()
                            ))
                        })
                })
                .transpose()?;
            let destination_kind = destination
                .map(|id| local_kind(locals, id, context))
                .transpose()?;
            if destination_kind != called_return {
                return Err(error(format!(
                    "{context} has a call destination with the wrong type"
                )));
            }
            require_block(blocks, *target, context)
        }
        LirTerminator::Branch {
            condition,
            if_true,
            if_false,
        } => {
            require_kind(
                LirValueKind::Bool,
                expression(condition)?,
                context,
                "branch",
            )?;
            require_block(blocks, *if_true, context)?;
            require_block(blocks, *if_false, context)
        }
        LirTerminator::Return { value } => {
            let actual = value.as_ref().map(&mut expression).transpose()?;
            if actual != return_kind {
                return Err(error(format!(
                    "{context} returns a value with the wrong type"
                )));
            }
            Ok(())
        }
        LirTerminator::Raise { .. } | LirTerminator::Unreachable => Ok(()),
    }
}

fn expression_kind(
    expression: &LirExpression,
    locals: &BTreeMap<LirLocalId, LirValueKind>,
    helpers: &BTreeSet<RuntimeHelper>,
    used_helpers: &mut BTreeSet<RuntimeHelper>,
    context: &str,
) -> Result<LirValueKind, CodegenError> {
    match expression {
        LirExpression::Value(LirValue::Local(id)) => local_kind(locals, *id, context),
        LirExpression::Value(LirValue::Bool(_)) => Ok(LirValueKind::Bool),
        LirExpression::Value(LirValue::Integer(_)) => Ok(LirValueKind::Integer),
        LirExpression::Unary { op, operand } => {
            let actual = expression_kind(operand, locals, helpers, used_helpers, context)?;
            let expected = match op {
                LirUnaryOp::Neg => LirValueKind::Integer,
                LirUnaryOp::Not => LirValueKind::Bool,
            };
            require_kind(expected.clone(), actual, context, "unary operation")?;
            Ok(expected)
        }
        LirExpression::Binary { op, left, right } => {
            let left = expression_kind(left, locals, helpers, used_helpers, context)?;
            let right = expression_kind(right, locals, helpers, used_helpers, context)?;
            let (operand, result) = match op {
                LirBinaryOp::And | LirBinaryOp::Or => (LirValueKind::Bool, LirValueKind::Bool),
                LirBinaryOp::Add | LirBinaryOp::Sub | LirBinaryOp::Mul => {
                    (LirValueKind::Integer, LirValueKind::Integer)
                }
                LirBinaryOp::Lt | LirBinaryOp::Le | LirBinaryOp::Gt | LirBinaryOp::Ge => {
                    (LirValueKind::Integer, LirValueKind::Bool)
                }
                LirBinaryOp::Eq | LirBinaryOp::Ne => {
                    if left != right {
                        return Err(error(format!(
                            "{context} compares values of different types"
                        )));
                    }
                    return Ok(LirValueKind::Bool);
                }
            };
            require_kind(operand.clone(), left, context, "binary operation")?;
            require_kind(operand, right, context, "binary operation")?;
            Ok(result)
        }
        LirExpression::RuntimeCall { helper, arguments } => {
            if !helpers.contains(helper) {
                return Err(error(format!(
                    "{context} uses undeclared helper {helper:?}"
                )));
            }
            used_helpers.insert(*helper);
            if *helper == RuntimeHelper::I32Rem {
                used_helpers.insert(RuntimeHelper::I32DivTrunc);
            }
            if arguments.len() != 2 {
                return Err(error(format!(
                    "{context} passes the wrong number of helper arguments"
                )));
            }
            for argument in arguments {
                let actual = expression_kind(argument, locals, helpers, used_helpers, context)?;
                require_kind(LirValueKind::Integer, actual, context, "helper argument")?;
            }
            Ok(LirValueKind::Integer)
        }
        LirExpression::Table { fields } => {
            let mut kinds = Vec::with_capacity(fields.len());
            for field in fields {
                kinds.push(expression_kind(
                    field,
                    locals,
                    helpers,
                    used_helpers,
                    context,
                )?);
            }
            Ok(LirValueKind::Table(kinds))
        }
        LirExpression::TableGet {
            table,
            index,
            result,
        } => {
            let actual = expression_kind(table, locals, helpers, used_helpers, context)?;
            if *index == 0 {
                return Err(error(format!("{context} uses zero as a table index")));
            }
            let LirValueKind::Table(fields) = actual else {
                return Err(error(format!(
                    "{context} has an invalid type in table indexing"
                )));
            };
            let Some(actual_result) = fields.get((*index - 1) as usize) else {
                return Err(error(format!(
                    "{context} indexes field {index} of a table with {} fields",
                    fields.len()
                )));
            };
            if actual_result != result {
                return Err(error(format!(
                    "{context} declares the wrong result type for table field {index}"
                )));
            }
            Ok(result.clone())
        }
        LirExpression::Enum {
            shapes,
            tag,
            fields,
        } => {
            let Some(shape) = shapes.get(*tag as usize) else {
                return Err(error(format!(
                    "{context} constructs an enum with tag {tag} outside its shape table"
                )));
            };
            if fields.len() != shape.len() {
                return Err(error(format!(
                    "{context} constructs an enum variant with the wrong field count"
                )));
            }
            for (field, expected) in fields.iter().zip(shape) {
                let actual = expression_kind(field, locals, helpers, used_helpers, context)?;
                require_kind(expected.clone(), actual, context, "enum field")?;
            }
            Ok(LirValueKind::Enum(shapes.clone()))
        }
        LirExpression::EnumTag { value } => {
            let actual = expression_kind(value, locals, helpers, used_helpers, context)?;
            if !matches!(actual, LirValueKind::Enum(_)) {
                return Err(error(format!(
                    "{context} reads the tag of a non-enum value"
                )));
            }
            Ok(LirValueKind::Integer)
        }
        LirExpression::EnumField {
            value,
            variant,
            field,
            result,
        } => {
            let actual = expression_kind(value, locals, helpers, used_helpers, context)?;
            let LirValueKind::Enum(shapes) = actual else {
                return Err(error(format!(
                    "{context} reads an enum field of a non-enum value"
                )));
            };
            let Some(shape) = shapes.get(*variant as usize) else {
                return Err(error(format!(
                    "{context} reads variant {variant} outside the enum shape table"
                )));
            };
            let Some(actual_result) = shape.get(*field as usize) else {
                return Err(error(format!(
                    "{context} reads field {field} beyond the variant shape"
                )));
            };
            if actual_result != result {
                return Err(error(format!(
                    "{context} declares the wrong result type for enum field {field}"
                )));
            }
            Ok(result.clone())
        }
    }
}

fn local_kind(
    locals: &BTreeMap<LirLocalId, LirValueKind>,
    id: LirLocalId,
    context: &str,
) -> Result<LirValueKind, CodegenError> {
    locals.get(&id).cloned().ok_or_else(|| {
        error(format!(
            "{context} references missing local v{}",
            id.index()
        ))
    })
}

fn require_block(
    blocks: &BTreeSet<LirBlockId>,
    id: LirBlockId,
    context: &str,
) -> Result<(), CodegenError> {
    if blocks.contains(&id) {
        Ok(())
    } else {
        Err(error(format!(
            "{context} references missing block bb{}",
            id.index()
        )))
    }
}

fn require_kind(
    expected: LirValueKind,
    actual: LirValueKind,
    context: &str,
    operation: &str,
) -> Result<(), CodegenError> {
    if expected == actual {
        Ok(())
    } else {
        Err(error(format!(
            "{context} has an invalid type in {operation}"
        )))
    }
}

fn error(detail: impl Into<String>) -> CodegenError {
    CodegenError::new(detail)
}
