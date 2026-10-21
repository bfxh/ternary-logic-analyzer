//! 三值逻辑核心：值、代数、AST、解析器、位并行表示与二进制互操作。

pub mod algebra;
pub mod ast;
pub mod bits;
pub mod bulk;
pub mod parser;
pub mod value;

pub use algebra::Algebra;
pub use ast::Expr;
pub use bits::{BitPlane, TriBit, TruthTable};
pub use bulk::TriVec;
pub use parser::Parser;
pub use value::Ternary;
