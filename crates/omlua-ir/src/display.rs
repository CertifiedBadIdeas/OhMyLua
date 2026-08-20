use std::fmt::{self, Write};

use crate::{
    AssertKind, BinaryOp, CheckedBinaryOp, Constant, LocalKind, OmEnum, OmFunction, OmProgram,
    OmStruct, OmType, Operand, Place, ProjectElem, RefKind, RefTarget, Rvalue, Statement, Terminator,
    UnaryOp, UnwindAction,
};

impl fmt::Display for OmProgram {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(formatter, "program entry @{}", self.entry)?;
        for definition in &self.structs {
            writeln!(formatter)?;
            write!(formatter, "{definition}")?;
        }
        for definition in &self.enums {
            writeln!(formatter)?;
            write!(formatter, "{definition}")?;
        }
        for function in &self.functions {
            writeln!(formatter)?;
            write!(formatter, "{function}")?;
        }
        Ok(())
    }
}

impl fmt::Display for OmStruct {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(formatter, "struct @{} {} {{", self.id, self.name)?;
        for field in &self.fields {
            writeln!(
                formatter,
                "  .{} {}: {}",
                field.id,
                field.name,
                type_name(field.ty)
            )?;
        }
        writeln!(formatter, "}}")
    }
}

impl fmt::Display for OmEnum {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(formatter, "enum @{} {} {{", self.id, self.name)?;
        for variant in &self.variants {
            if variant.fields.is_empty() {
                writeln!(formatter, "  v{} {}", variant.id, variant.name)?;
            } else {
                writeln!(formatter, "  v{} {} {{", variant.id, variant.name)?;
                for field in &variant.fields {
                    writeln!(
                        formatter,
                        "    .{} {}: {}",
                        field.id,
                        field.name,
                        type_name(field.ty)
                    )?;
                }
                writeln!(formatter, "  }}")?;
            }
        }
        writeln!(formatter, "}}")
    }
}

impl fmt::Display for OmFunction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "fn @{} {}(", self.id, self.name)?;
        for (index, parameter) in self.parameters.iter().enumerate() {
            if index != 0 {
                write!(formatter, ", ")?;
            }
            write!(formatter, "%{parameter}")?;
        }
        writeln!(formatter, ") -> {} {{", type_name(self.return_type))?;

        writeln!(formatter, "  locals:")?;
        for local in &self.locals {
            writeln!(
                formatter,
                "    %{}: {} {}",
                local.id,
                type_name(local.ty),
                local_kind_name(local.kind)
            )?;
        }

        for block in &self.blocks {
            writeln!(formatter, "  bb{}:", block.id)?;
            for statement in &block.statements {
                writeln!(formatter, "    {}", format_statement(statement))?;
            }
            writeln!(formatter, "    {}", format_terminator(&block.terminator))?;
        }
        writeln!(formatter, "}}")
    }
}

fn type_name(ty: OmType) -> String {
    match ty {
        OmType::Unit => "unit".to_owned(),
        OmType::Bool => "bool".to_owned(),
        OmType::I32 => "i32".to_owned(),
        OmType::Struct(id) => format!("struct @{id}"),
        OmType::Enum(id) => format!("enum @{id}"),
        OmType::Ref { kind, target } => format!(
            "{}{}",
            match kind {
                RefKind::Shared => "&",
                RefKind::Mutable => "&mut ",
            },
            ref_target_name(target)
        ),
    }
}

fn ref_target_name(target: RefTarget) -> String {
    match target {
        RefTarget::Unit => "unit".to_owned(),
        RefTarget::Bool => "bool".to_owned(),
        RefTarget::I32 => "i32".to_owned(),
        RefTarget::Struct(id) => format!("struct @{id}"),
        RefTarget::Enum(id) => format!("enum @{id}"),
    }
}

fn local_kind_name(kind: LocalKind) -> &'static str {
    match kind {
        LocalKind::Return => "return",
        LocalKind::Parameter => "parameter",
        LocalKind::Temporary => "temporary",
        LocalKind::CheckedValue => "checked-value",
        LocalKind::CheckedOverflow => "checked-overflow",
        LocalKind::Discriminant => "discriminant",
    }
}

fn format_statement(statement: &Statement) -> String {
    match statement {
        Statement::Assign { destination, value } => {
            format!("%{destination} = {}", format_rvalue(value))
        }
        Statement::Store { destination, value } => {
            format!("{} = {}", format_place(destination), format_rvalue(value))
        }
        Statement::CheckedBinary {
            value,
            overflow,
            op,
            left,
            right,
        } => format!(
            "(%{value}, %{overflow}) = checked_{} {}, {}",
            checked_binary_name(*op),
            format_operand(left),
            format_operand(right)
        ),
    }
}

fn format_rvalue(value: &Rvalue) -> String {
    match value {
        Rvalue::Use(operand) => format_operand(operand),
        Rvalue::Unary { op, operand } => {
            format!("{} {}", unary_name(*op), format_operand(operand))
        }
        Rvalue::Binary { op, left, right } => format!(
            "{} {}, {}",
            binary_name(*op),
            format_operand(left),
            format_operand(right)
        ),
        Rvalue::Struct { ty, fields } => format!(
            "struct @{ty} {{ {} }}",
            fields
                .iter()
                .map(format_operand)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Rvalue::Variant {
            ty,
            variant,
            fields,
        } => format!(
            "variant @{ty}#{variant} {{ {} }}",
            fields
                .iter()
                .map(format_operand)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Rvalue::Discriminant { source } => {
            format!("discriminant {}", format_operand(source))
        }
        Rvalue::Borrow { kind, source } => format!(
            "borrow_{} {}",
            match kind {
                RefKind::Shared => "shared",
                RefKind::Mutable => "mut",
            },
            format_place(source)
        ),
    }
}

fn format_terminator(terminator: &Terminator) -> String {
    match terminator {
        Terminator::Goto { target } => format!("goto bb{target}"),
        Terminator::SwitchInt {
            discriminant,
            targets,
            otherwise,
        } => {
            let mut text = format!("switch {} [", format_operand(discriminant));
            for (index, (value, target)) in targets.iter().enumerate() {
                if index != 0 {
                    text.push_str(", ");
                }
                write!(text, "{}: bb{}", value.0, target).expect("writing to a String cannot fail");
            }
            if !targets.is_empty() {
                text.push_str(", ");
            }
            write!(text, "otherwise: bb{otherwise}]").expect("writing to a String cannot fail");
            text
        }
        Terminator::Call {
            callee,
            arguments,
            destination,
            target,
            unwind,
        } => {
            let arguments = arguments
                .iter()
                .map(format_operand)
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "{} = call @{callee}({arguments}) -> bb{target} unwind {}",
                format_place(destination),
                unwind_name(*unwind)
            )
        }
        Terminator::Assert {
            condition,
            expected,
            kind,
            target,
            unwind,
        } => format!(
            "assert {} == {expected} {} -> bb{target} unwind {}",
            format_operand(condition),
            format_assert_kind(kind),
            unwind_name(*unwind)
        ),
        Terminator::Return => "return".to_owned(),
        Terminator::Unreachable => "unreachable".to_owned(),
    }
}

fn format_assert_kind(kind: &AssertKind) -> String {
    match kind {
        AssertKind::Overflow { op, left, right } => format!(
            "overflow_{}({}, {})",
            binary_name(*op),
            format_operand(left),
            format_operand(right)
        ),
        AssertKind::OverflowNeg(operand) => {
            format!("overflow_neg({})", format_operand(operand))
        }
        AssertKind::DivisionByZero(operand) => {
            format!("division_by_zero({})", format_operand(operand))
        }
        AssertKind::RemainderByZero(operand) => {
            format!("remainder_by_zero({})", format_operand(operand))
        }
    }
}

fn format_operand(operand: &Operand) -> String {
    match operand {
        Operand::Copy(local) => format!("copy %{local}"),
        Operand::Move(local) => format!("move %{local}"),
        Operand::Project { base, path, moved } => {
            let place = format_projected_place(*base, path);
            format!("{} {place}", if *moved { "move" } else { "copy" })
        }
        Operand::Constant(constant) => match constant {
            Constant::Unit => "unit".to_owned(),
            Constant::Bool(value) => value.to_string(),
            Constant::I32(value) => format!("{value}_i32"),
        },
    }
}

fn format_place(place: &Place) -> String {
    format_projected_place(place.base, &place.path)
}

fn format_projected_place(base: crate::LocalId, path: &[ProjectElem]) -> String {
    let mut place = format!("%{base}");
    for (index, element) in path.iter().enumerate() {
        match element {
            ProjectElem::Deref => {
                if index + 1 < path.len() {
                    place = format!("(*{place})");
                } else {
                    place = format!("*{place}");
                }
            }
            ProjectElem::Downcast(variant) => {
                write!(place, "#{variant}").expect("writing to a String cannot fail");
            }
            ProjectElem::Field(field) => {
                write!(place, ".{field}").expect("writing to a String cannot fail");
            }
        }
    }
    place
}

fn unary_name(op: UnaryOp) -> &'static str {
    match op {
        UnaryOp::Neg => "neg",
        UnaryOp::Not => "not",
        UnaryOp::BitNot => "bit_not",
    }
}

fn binary_name(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::And => "and",
        BinaryOp::Add => "add",
        BinaryOp::Sub => "sub",
        BinaryOp::Mul => "mul",
        BinaryOp::Div => "div",
        BinaryOp::Rem => "rem",
        BinaryOp::Eq => "eq",
        BinaryOp::Ne => "ne",
        BinaryOp::Lt => "lt",
        BinaryOp::Le => "le",
        BinaryOp::Gt => "gt",
        BinaryOp::Ge => "ge",
    }
}

fn checked_binary_name(op: CheckedBinaryOp) -> &'static str {
    match op {
        CheckedBinaryOp::Add => "add",
        CheckedBinaryOp::Sub => "sub",
        CheckedBinaryOp::Mul => "mul",
    }
}

fn unwind_name(action: UnwindAction) -> String {
    match action {
        UnwindAction::Continue => "continue".to_owned(),
        UnwindAction::Unreachable => "unreachable".to_owned(),
        UnwindAction::TerminateAbi => "terminate-abi".to_owned(),
        UnwindAction::TerminateInCleanup => "terminate-cleanup".to_owned(),
        UnwindAction::Cleanup(block) => format!("cleanup bb{block}"),
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        AssertKind, BinaryOp, BlockId, Constant, FieldId, FunctionId, LocalId, LocalKind, OmBlock,
        OmEnum, OmField, OmFunction, OmLocal, OmProgram, OmType, OmVariant, Operand, ProjectElem,
        Rvalue, Statement, SwitchValue, Terminator, TypeId, UnwindAction, VariantId,
    };

    #[test]
    fn formats_program_deterministically() {
        let program = OmProgram {
            entry: FunctionId::new(0),
            structs: vec![],
            enums: vec![],
            functions: vec![OmFunction {
                id: FunctionId::new(0),
                name: "omlua_input::main".to_owned(),
                return_type: OmType::Unit,
                parameters: vec![],
                locals: vec![
                    OmLocal {
                        id: LocalId::new(0),
                        ty: OmType::Unit,
                        kind: LocalKind::Return,
                    },
                    OmLocal {
                        id: LocalId::new(1),
                        ty: OmType::I32,
                        kind: LocalKind::Temporary,
                    },
                    OmLocal {
                        id: LocalId::new(2),
                        ty: OmType::Bool,
                        kind: LocalKind::Temporary,
                    },
                ],
                blocks: vec![
                    OmBlock {
                        id: BlockId::new(0),
                        statements: vec![Statement::Assign {
                            destination: LocalId::new(2),
                            value: Rvalue::Binary {
                                op: BinaryOp::Ge,
                                left: Operand::Copy(LocalId::new(1)),
                                right: Operand::Constant(Constant::I32(0)),
                            },
                        }],
                        terminator: Terminator::SwitchInt {
                            discriminant: Operand::Move(LocalId::new(2)),
                            targets: vec![(SwitchValue(0), BlockId::new(2))],
                            otherwise: BlockId::new(1),
                        },
                    },
                    OmBlock {
                        id: BlockId::new(1),
                        statements: vec![],
                        terminator: Terminator::Assert {
                            condition: Operand::Copy(LocalId::new(2)),
                            expected: false,
                            kind: AssertKind::OverflowNeg(Operand::Copy(LocalId::new(1))),
                            target: BlockId::new(2),
                            unwind: UnwindAction::Continue,
                        },
                    },
                    OmBlock {
                        id: BlockId::new(2),
                        statements: vec![],
                        terminator: Terminator::Return,
                    },
                ],
            }],
        };

        assert_eq!(
            program.to_string(),
            concat!(
                "program entry @0\n",
                "\n",
                "fn @0 omlua_input::main() -> unit {\n",
                "  locals:\n",
                "    %0: unit return\n",
                "    %1: i32 temporary\n",
                "    %2: bool temporary\n",
                "  bb0:\n",
                "    %2 = ge copy %1, 0_i32\n",
                "    switch move %2 [0: bb2, otherwise: bb1]\n",
                "  bb1:\n",
                "    assert copy %2 == false overflow_neg(copy %1) -> bb2 unwind continue\n",
                "  bb2:\n",
                "    return\n",
                "}\n",
            )
        );
    }

    #[test]
    fn formats_enums_and_match_deterministically() {
        let command = TypeId::new(0);
        let program = OmProgram {
            entry: FunctionId::new(0),
            structs: vec![],
            enums: vec![OmEnum {
                id: command,
                name: "omlua_input::Command".to_owned(),
                variants: vec![
                    OmVariant {
                        id: VariantId::new(0),
                        name: "Stop".to_owned(),
                        fields: vec![],
                    },
                    OmVariant {
                        id: VariantId::new(1),
                        name: "GoTo".to_owned(),
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
                    },
                    OmVariant {
                        id: VariantId::new(2),
                        name: "SetThrottle".to_owned(),
                        fields: vec![OmField {
                            id: FieldId::new(0),
                            name: "0".to_owned(),
                            ty: OmType::I32,
                        }],
                    },
                ],
            }],
            functions: vec![OmFunction {
                id: FunctionId::new(0),
                name: "omlua_input::main".to_owned(),
                return_type: OmType::Unit,
                parameters: vec![],
                locals: vec![
                    OmLocal {
                        id: LocalId::new(0),
                        ty: OmType::Unit,
                        kind: LocalKind::Return,
                    },
                    OmLocal {
                        id: LocalId::new(1),
                        ty: OmType::Enum(command),
                        kind: LocalKind::Temporary,
                    },
                    OmLocal {
                        id: LocalId::new(2),
                        ty: OmType::I32,
                        kind: LocalKind::Discriminant,
                    },
                    OmLocal {
                        id: LocalId::new(3),
                        ty: OmType::I32,
                        kind: LocalKind::Temporary,
                    },
                ],
                blocks: vec![OmBlock {
                    id: BlockId::new(0),
                    statements: vec![
                        Statement::Assign {
                            destination: LocalId::new(1),
                            value: Rvalue::Variant {
                                ty: command,
                                variant: VariantId::new(1),
                                fields: vec![
                                    Operand::Constant(Constant::I32(20)),
                                    Operand::Constant(Constant::I32(22)),
                                ],
                            },
                        },
                        Statement::Assign {
                            destination: LocalId::new(2),
                            value: Rvalue::Discriminant {
                                source: Operand::Copy(LocalId::new(1)),
                            },
                        },
                        Statement::Assign {
                            destination: LocalId::new(3),
                            value: Rvalue::Use(Operand::Project {
                                base: LocalId::new(1),
                                path: vec![
                                    ProjectElem::Downcast(VariantId::new(1)),
                                    ProjectElem::Field(FieldId::new(0)),
                                ],
                                moved: true,
                            }),
                        },
                    ],
                    terminator: Terminator::Return,
                }],
            }],
        };

        assert_eq!(
            program.to_string(),
            concat!(
                "program entry @0\n",
                "\n",
                "enum @0 omlua_input::Command {\n",
                "  v0 Stop\n",
                "  v1 GoTo {\n",
                "    .0 x: i32\n",
                "    .1 y: i32\n",
                "  }\n",
                "  v2 SetThrottle {\n",
                "    .0 0: i32\n",
                "  }\n",
                "}\n",
                "\n",
                "fn @0 omlua_input::main() -> unit {\n",
                "  locals:\n",
                "    %0: unit return\n",
                "    %1: enum @0 temporary\n",
                "    %2: i32 discriminant\n",
                "    %3: i32 temporary\n",
                "  bb0:\n",
                "    %1 = variant @0#1 { 20_i32, 22_i32 }\n",
                "    %2 = discriminant copy %1\n",
                "    %3 = move %1#1.0\n",
                "    return\n",
                "}\n",
            )
        );
    }
}
