use cascade_core::span::SourceSpan;

/// A scenario file that cannot be parsed or does not match the model.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ScenarioError {
    #[error("scenario files are not supported yet")]
    NotImplemented,
    #[error("{span}: {message}")]
    Invalid { message: String, span: SourceSpan },
}

/// The simulator could not run a scenario against a model.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SimError {
    #[error("the simulator is not implemented yet")]
    NotImplemented,
    #[error("unknown instance `{0}`")]
    UnknownInstance(String),
    #[error("unknown machine `{0}`")]
    UnknownMachine(String),
    #[error("unknown external source `{0}`")]
    UnknownSource(String),
    #[error("source `{external}` cannot fire `{trigger}`")]
    SourceCannotFire { external: String, trigger: String },
    #[error("the cascade did not settle within {0} steps; is there an unbounded cycle?")]
    StepLimit(usize),
    #[error("the finding is not a race candidate")]
    NotARace,
}
