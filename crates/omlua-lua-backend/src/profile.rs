#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LuaDialect {
    Lua54,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NumericModel {
    pub integer_bits: u8,
    pub float_bits: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlFlowCapabilities {
    pub label_jumps: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OperatorCapabilities {
    pub native_bitwise: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LuaBackendProfile {
    dialect: LuaDialect,
    numeric: NumericModel,
    control_flow: ControlFlowCapabilities,
    operators: OperatorCapabilities,
}

impl LuaBackendProfile {
    pub const fn lua54() -> Self {
        Self {
            dialect: LuaDialect::Lua54,
            numeric: NumericModel {
                integer_bits: 64,
                float_bits: 64,
            },
            control_flow: ControlFlowCapabilities { label_jumps: true },
            operators: OperatorCapabilities {
                native_bitwise: true,
            },
        }
    }

    pub const fn dialect(&self) -> LuaDialect {
        self.dialect
    }

    pub const fn numeric(&self) -> NumericModel {
        self.numeric
    }

    pub const fn control_flow(&self) -> ControlFlowCapabilities {
        self.control_flow
    }

    pub const fn operators(&self) -> OperatorCapabilities {
        self.operators
    }
}
