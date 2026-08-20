use omlua_ir::{
    BlockId, Constant, FieldId, FunctionId, LocalId, LocalKind, OmBlock, OmField, OmFunction,
    OmLocal, OmProgram, OmStruct, OmType, Operand, ProjectElem, Rvalue, Statement, Terminator,
    TypeId,
};

#[test]
fn formats_structs_and_shared_references_deterministically() {
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
                OmLocal {
                    id: LocalId::new(0),
                    ty: OmType::Unit,
                    kind: LocalKind::Return,
                },
                OmLocal {
                    id: LocalId::new(1),
                    ty: OmType::Struct(point),
                    kind: LocalKind::Temporary,
                },
                OmLocal {
                    id: LocalId::new(2),
                    ty: OmType::Ref { kind: omlua_ir::RefKind::Shared, target: omlua_ir::RefTarget::Struct(point) },
                    kind: LocalKind::Temporary,
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
                        value: Rvalue::Borrow { kind: omlua_ir::RefKind::Shared, source: omlua_ir::Place::local(LocalId::new(1)) },
                    },
                    Statement::Assign {
                        destination: LocalId::new(3),
                        value: Rvalue::Use(Operand::Project {
                            base: LocalId::new(2),
                            path: vec![ProjectElem::Deref, ProjectElem::Field(FieldId::new(0))],
                            moved: false,
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
            "struct @0 Point {\n",
            "  .0 x: i32\n",
            "  .1 y: i32\n",
            "}\n",
            "\n",
            "fn @0 main() -> unit {\n",
            "  locals:\n",
            "    %0: unit return\n",
            "    %1: struct @0 temporary\n",
            "    %2: &struct @0 temporary\n",
            "    %3: i32 temporary\n",
            "  bb0:\n",
            "    %1 = struct @0 { 20_i32, 22_i32 }\n",
            "    %2 = borrow_shared %1\n",
            "    %3 = copy (*%2).0\n",
            "    return\n",
            "}\n",
        )
    );
}
