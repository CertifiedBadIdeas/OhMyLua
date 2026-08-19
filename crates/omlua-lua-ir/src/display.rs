use std::fmt;

use crate::*;

impl fmt::Display for LirProgram {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            formatter,
            "lir entry @{} requirements(integer_bits={}, label_jumps={})",
            self.entry, self.requirements.minimum_integer_bits, self.requirements.label_jumps,
        )?;
        for function in &self.functions {
            writeln!(formatter)?;
            function.fmt(formatter)?;
        }
        Ok(())
    }
}

impl fmt::Display for LirFunction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "fn @{}(", self.id)?;
        write_joined(formatter, &self.parameters, |formatter, parameter| {
            write!(formatter, "%{parameter}")
        })?;
        match self.return_local {
            Some(local) => writeln!(formatter, ") -> %{local} {{")?,
            None => writeln!(formatter, ") -> unit {{")?,
        }
        writeln!(formatter, "  locals:")?;
        for local in &self.locals {
            write!(formatter, "    %{}: {}", local.id, local.kind)?;
            if local.parameter {
                write!(formatter, " parameter")?;
            }
            writeln!(formatter)?;
        }
        for block in &self.blocks {
            writeln!(formatter, "  bb{}:", block.id)?;
            for statement in &block.statements {
                writeln!(formatter, "    {statement}")?;
            }
            writeln!(formatter, "    {}", block.terminator)?;
        }
        writeln!(formatter, "}}")
    }
}

impl fmt::Display for LirValueKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bool => formatter.write_str("bool"),
            Self::Integer => formatter.write_str("integer"),
            Self::Table(fields) => {
                formatter.write_str("table<")?;
                write_joined(formatter, fields, |formatter, field| field.fmt(formatter))?;
                formatter.write_str(">")
            }
            Self::Enum(shapes) => {
                formatter.write_str("enum<")?;
                write_joined(formatter, shapes, |formatter, shape| {
                    formatter.write_str("[")?;
                    write_joined(formatter, shape, |formatter, field| field.fmt(formatter))?;
                    formatter.write_str("]")
                })?;
                formatter.write_str(">")
            }
        }
    }
}

impl fmt::Display for LirStatement {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Assign { destination, value } => write!(formatter, "%{destination} = {value}"),
        }
    }
}

impl fmt::Display for LirExpression {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Value(value) => value.fmt(formatter),
            Self::Unary { op, operand } => write!(formatter, "({op} {operand})"),
            Self::Binary { op, left, right } => write!(formatter, "({op} {left}, {right})"),
            Self::RuntimeCall { helper, arguments } => {
                write!(formatter, "runtime {helper}(")?;
                write_joined(formatter, arguments, |formatter, argument| {
                    argument.fmt(formatter)
                })?;
                formatter.write_str(")")
            }
            Self::Table {
                fields,
            } => {
                formatter.write_str("table {")?;
                write_joined(formatter, fields, |formatter, field| field.fmt(formatter))?;
                formatter.write_str("}")
            }
            Self::TableGet {
                table,
                index,
                result,
            } => write!(formatter, "table_get {table}[{index}] -> {result}"),
            Self::Enum { tag, fields, .. } => {
                write!(formatter, "enum #{tag} {{")?;
                write_joined(formatter, fields, |formatter, field| field.fmt(formatter))?;
                formatter.write_str("}")
            }
            Self::EnumTag { value } => write!(formatter, "enum_tag {value}"),
            Self::EnumField {
                value,
                variant,
                field,
                result,
            } => write!(formatter, "enum_field {value}#{variant}.{field} -> {result}"),
        }
    }
}

impl fmt::Display for LirValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local(local) => write!(formatter, "%{local}"),
            Self::Bool(value) => value.fmt(formatter),
            Self::Integer(value) => value.fmt(formatter),
        }
    }
}

impl fmt::Display for LirUnaryOp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Neg => "neg",
            Self::Not => "not",
        })
    }
}

impl fmt::Display for LirBinaryOp {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::And => "and",
            Self::Or => "or",
            Self::Add => "add",
            Self::Sub => "sub",
            Self::Mul => "mul",
            Self::Eq => "eq",
            Self::Ne => "ne",
            Self::Lt => "lt",
            Self::Le => "le",
            Self::Gt => "gt",
            Self::Ge => "ge",
        })
    }
}

impl fmt::Display for RuntimeHelper {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::I32DivTrunc => "i32_div_trunc",
            Self::I32Rem => "i32_rem",
        })
    }
}

impl fmt::Display for LirTerminator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Jump { target } => write!(formatter, "jump bb{target}"),
            Self::Switch {
                discriminant,
                targets,
                otherwise,
            } => {
                write!(formatter, "switch {discriminant} [")?;
                write_joined(formatter, targets, |formatter, (value, target)| {
                    write!(formatter, "{value}: bb{target}")
                })?;
                write!(formatter, ", otherwise: bb{otherwise}]")
            }
            Self::Call {
                callee,
                arguments,
                destination,
                target,
            } => {
                if let Some(destination) = destination {
                    write!(formatter, "%{destination} = ")?;
                }
                write!(formatter, "call @{callee}(")?;
                write_joined(formatter, arguments, |formatter, argument| {
                    argument.fmt(formatter)
                })?;
                write!(formatter, ") -> bb{target}")
            }
            Self::Branch {
                condition,
                if_true,
                if_false,
            } => {
                write!(
                    formatter,
                    "branch {condition} [true: bb{if_true}, false: bb{if_false}]"
                )
            }
            Self::Return { value: Some(value) } => write!(formatter, "return {value}"),
            Self::Return { value: None } => formatter.write_str("return"),
            Self::Raise { message } => write!(formatter, "raise {message:?}"),
            Self::Unreachable => formatter.write_str("unreachable"),
        }
    }
}

fn write_joined<T>(
    formatter: &mut fmt::Formatter<'_>,
    values: &[T],
    mut write_value: impl FnMut(&mut fmt::Formatter<'_>, &T) -> fmt::Result,
) -> fmt::Result {
    for (index, value) in values.iter().enumerate() {
        if index > 0 {
            formatter.write_str(", ")?;
        }
        write_value(formatter, value)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_program_deterministically() {
        assert_eq!(
            scalar_return_program().to_string(),
            concat!(
                "lir entry @0 requirements(integer_bits=64, label_jumps=true)\n",
                "\n",
                "fn @0(%1) -> %0 {\n",
                "  locals:\n",
                "    %0: integer\n",
                "    %1: integer parameter\n",
                "  bb0:\n",
                "    %0 = (add %1, 1)\n",
                "    return %0\n",
                "}\n",
            )
        );
    }

    fn scalar_return_program() -> LirProgram {
        let return_local = LirLocalId::new(0);
        let parameter = LirLocalId::new(1);
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
                parameters: vec![parameter],
                return_local: Some(return_local),
                locals: vec![
                    LirLocal {
                        id: return_local,
                        kind: LirValueKind::Integer,
                        parameter: false,
                    },
                    LirLocal {
                        id: parameter,
                        kind: LirValueKind::Integer,
                        parameter: true,
                    },
                ],
                blocks: vec![LirBlock {
                    id: LirBlockId::new(0),
                    statements: vec![LirStatement::Assign {
                        destination: return_local,
                        value: LirExpression::Binary {
                            op: LirBinaryOp::Add,
                            left: Box::new(LirExpression::Value(LirValue::Local(parameter))),
                            right: Box::new(LirExpression::Value(LirValue::Integer(1))),
                        },
                    }],
                    terminator: LirTerminator::Return {
                        value: Some(LirExpression::Value(LirValue::Local(return_local))),
                    },
                }],
            }],
        }
    }

    #[test]
    fn formats_enums_deterministically() {
        assert_eq!(
            enum_program().to_string(),
            concat!(
                "lir entry @0 requirements(integer_bits=64, label_jumps=true)\n",
                "\n",
                "fn @0(%1) -> %0 {\n",
                "  locals:\n",
                "    %0: integer\n",
                "    %1: enum<[integer], [integer, integer]> parameter\n",
                "    %2: integer\n",
                "    %3: enum<[integer], [integer, integer]>\n",
                "  bb0:\n",
                "    %3 = enum #1 {20, 22}\n",
                "    %2 = enum_tag %3\n",
                "    %0 = enum_field %3#1.0 -> integer\n",
                "    return %0\n",
                "}\n",
            )
        );
    }

    fn enum_program() -> LirProgram {
        let shapes = vec![
            vec![LirValueKind::Integer],
            vec![LirValueKind::Integer, LirValueKind::Integer],
        ];
        let return_local = LirLocalId::new(0);
        let parameter = LirLocalId::new(1);
        let tag_local = LirLocalId::new(2);
        let value_local = LirLocalId::new(3);
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
                parameters: vec![parameter],
                return_local: Some(return_local),
                locals: vec![
                    LirLocal {
                        id: return_local,
                        kind: LirValueKind::Integer,
                        parameter: false,
                    },
                    LirLocal {
                        id: parameter,
                        kind: LirValueKind::Enum(shapes.clone()),
                        parameter: true,
                    },
                    LirLocal {
                        id: tag_local,
                        kind: LirValueKind::Integer,
                        parameter: false,
                    },
                    LirLocal {
                        id: value_local,
                        kind: LirValueKind::Enum(shapes),
                        parameter: false,
                    },
                ],
                blocks: vec![LirBlock {
                    id: LirBlockId::new(0),
                    statements: vec![
                        LirStatement::Assign {
                            destination: value_local,
                            value: LirExpression::Enum {
                                shapes: enum_shapes(),
                                tag: 1,
                                fields: vec![integer_expression(20), integer_expression(22)],
                            },
                        },
                        LirStatement::Assign {
                            destination: tag_local,
                            value: LirExpression::EnumTag {
                                value: Box::new(local_expression(value_local)),
                            },
                        },
                        LirStatement::Assign {
                            destination: return_local,
                            value: LirExpression::EnumField {
                                value: Box::new(local_expression(value_local)),
                                variant: 1,
                                field: 0,
                                result: LirValueKind::Integer,
                            },
                        },
                    ],
                    terminator: LirTerminator::Return {
                        value: Some(local_expression(return_local)),
                    },
                }],
            }],
        }
    }

    fn enum_shapes() -> Vec<Vec<LirValueKind>> {
        vec![
            vec![LirValueKind::Integer],
            vec![LirValueKind::Integer, LirValueKind::Integer],
        ]
    }

    fn local_expression(id: LirLocalId) -> LirExpression {
        LirExpression::Value(LirValue::Local(id))
    }

    fn integer_expression(value: i64) -> LirExpression {
        LirExpression::Value(LirValue::Integer(value))
    }
}
