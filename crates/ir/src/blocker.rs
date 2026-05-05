use std::fmt;

/// A single construct kind that prevented a function from being lowered.
/// One variant per distinct lowering decision; `BTreeSet<BlockerKind>` gives
/// the full set of constructs blocking a given function body.
///
/// `Ord` is required for `BTreeSet` membership; ordering is arbitrary but
/// deterministic, so reports diff cleanly across runs.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BlockerKind {
    // --- Param blockers ---
    /// `...rest` parameter
    RestParam,
    /// Destructuring parameter: `{a, b}` or `[a, b]`
    DestructuringParam,
    /// Default-value parameter: `x = 5`
    DefaultParam,

    // --- Statement blockers ---
    /// `throw expr`
    ThrowStmt,
    /// `break` (unlabeled or labeled)
    BreakStmt,
    /// `switch (x) { case ... }`
    SwitchStmt,
    /// `do { } while (cond)`
    DoWhileStmt,
    /// `for (x in obj)`
    ForInStmt,
    /// `label: stmt`
    LabeledStmt,
    /// `continue label` (labeled continue; plain continue is fine)
    LabeledContinue,
    /// `debugger`
    DebuggerStmt,
    /// `class Foo { ... }` inside a function body
    ClassDecl,
    /// Any other statement kind not recognized above
    UnsupportedStmt,

    // --- For-of specific blockers ---
    /// `for await (x of ...)`
    ForAwaitOf,
    /// For-of with destructuring binding: `for (const {a} of ...)`
    ForOfDestructuringBinding,
    /// For-of body contains a bare `return` (breaks iterator protocol)
    ForOfBareReturn,

    // --- Try-specific blockers ---
    /// `catch ({message})` — destructuring catch param
    TryCatchDestructuringParam,

    // --- Expression blockers ---
    /// Comma operator: `a, b`
    SequenceExpr,
    /// Plain `=` assignment in expression position: `f(x = 1)`, `if (a = b) {}`.
    /// Distinct from statement-level assignment, which is lowerable.
    AssignmentExpr,
    /// Compound `+=`, `-=`, etc. in expression position. Lowering must
    /// desugar to `x = x op e` while preserving target side-effects, so
    /// it's a different decision from plain `AssignmentExpr`.
    CompoundAssignmentExpr,
    /// Nullish coalescing: `a ?? b`
    NullishCoalescing,
    /// Logical-assign: `&&=`, `||=`, `??=`
    LogicalAssignOp,
    /// Tagged template: `` tag`...` ``
    TaggedTemplateExpr,
    /// Optional chaining: `a?.b`
    ChainExpr,
    /// `await expr`
    AwaitExpr,
    /// `yield expr`
    YieldExpr,
    /// Dynamic import: `import(src)`
    ImportExpr,
    /// Regular expression literal: `/re/g`
    RegExpLiteral,
    /// BigInt literal: `42n`
    BigIntLiteral,
    /// `this`
    ThisExpr,
    /// `import.meta` / `new.target`
    MetaProperty,
    /// Array hole / elision: `[1,,3]`
    ArrayElision,
    /// Binary operator not in the lowering whitelist (e.g. `in`, `instanceof`, `**`)
    UnsupportedBinOp,
    /// Unary operator not in the lowering whitelist (e.g. `delete`)
    UnsupportedUnaryOp,
    /// Any other expression kind not recognized above
    UnsupportedExpr,

    // --- Variable declaration blockers ---
    /// `let a = 1, b = 2` — more than one declarator
    MultipleDeclarators,
    /// `const {a, b} = x` — destructuring binding
    DestructuringBinding,
    /// `using x = ...` or other exotic declaration kind
    UnsupportedVarKind,

    // --- Function-shape blockers ---
    /// `async function` or `async () =>`
    AsyncFunction,
    /// `function*` generator
    GeneratorFunction,
    /// Anonymous nested function declaration (illegal JS anyway)
    AnonymousFunctionDecl,
}

impl fmt::Display for BlockerKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::RestParam => "RestParam",
            Self::DestructuringParam => "DestructuringParam",
            Self::DefaultParam => "DefaultParam",
            Self::ThrowStmt => "ThrowStmt",
            Self::BreakStmt => "BreakStmt",
            Self::SwitchStmt => "SwitchStmt",
            Self::DoWhileStmt => "DoWhileStmt",
            Self::ForInStmt => "ForInStmt",
            Self::LabeledStmt => "LabeledStmt",
            Self::LabeledContinue => "LabeledContinue",
            Self::DebuggerStmt => "DebuggerStmt",
            Self::ClassDecl => "ClassDecl",
            Self::UnsupportedStmt => "UnsupportedStmt",
            Self::ForAwaitOf => "ForAwaitOf",
            Self::ForOfDestructuringBinding => "ForOfDestructuringBinding",
            Self::ForOfBareReturn => "ForOfBareReturn",
            Self::TryCatchDestructuringParam => "TryCatchDestructuringParam",
            Self::SequenceExpr => "SequenceExpr",
            Self::AssignmentExpr => "AssignmentExpr",
            Self::CompoundAssignmentExpr => "CompoundAssignmentExpr",
            Self::NullishCoalescing => "NullishCoalescing",
            Self::LogicalAssignOp => "LogicalAssignOp",
            Self::TaggedTemplateExpr => "TaggedTemplateExpr",
            Self::ChainExpr => "ChainExpr",
            Self::AwaitExpr => "AwaitExpr",
            Self::YieldExpr => "YieldExpr",
            Self::ImportExpr => "ImportExpr",
            Self::RegExpLiteral => "RegExpLiteral",
            Self::BigIntLiteral => "BigIntLiteral",
            Self::ThisExpr => "ThisExpr",
            Self::MetaProperty => "MetaProperty",
            Self::ArrayElision => "ArrayElision",
            Self::UnsupportedBinOp => "UnsupportedBinOp",
            Self::UnsupportedUnaryOp => "UnsupportedUnaryOp",
            Self::UnsupportedExpr => "UnsupportedExpr",
            Self::MultipleDeclarators => "MultipleDeclarators",
            Self::DestructuringBinding => "DestructuringBinding",
            Self::UnsupportedVarKind => "UnsupportedVarKind",
            Self::AsyncFunction => "AsyncFunction",
            Self::GeneratorFunction => "GeneratorFunction",
            Self::AnonymousFunctionDecl => "AnonymousFunctionDecl",
        };
        write!(f, "{s}")
    }
}
