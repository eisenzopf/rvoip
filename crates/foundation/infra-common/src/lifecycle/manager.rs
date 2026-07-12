use crate::errors::types::{Error, Result};
use crate::lifecycle::component::{Component, ComponentState};
use crate::lifecycle::dependency::DependencyGraph;
use std::collections::HashMap;
use std::fmt::{self, Debug, Formatter};
use std::sync::{Arc, RwLock};
use thiserror::Error;
use tokio::sync::Mutex;

/// Errors related to lifecycle management
#[derive(Error, Debug)]
pub enum LifecycleError {
    #[error("Component not found: {0}")]
    ComponentNotFound(String),

    #[error("Component already exists: {0}")]
    ComponentAlreadyExists(String),

    #[error("Component is in the wrong state: {0} (expected {1:?}, found {2:?})")]
    InvalidState(String, ComponentState, ComponentState),

    #[error("Circular dependency detected: {0}")]
    CircularDependency(String),

    #[error("Dependency not found: {0} required by {1}")]
    DependencyNotFound(String, String),

    #[error("Operation timeout: {0}")]
    Timeout(String),

    #[error("Lifecycle error: {0}")]
    Other(String),
}

impl From<LifecycleError> for Error {
    fn from(err: LifecycleError) -> Self {
        Error::Component(err.to_string())
    }
}

/// Type alias for a boxed component
pub type BoxedComponent = Box<dyn Component>;

/// Type alias for an Arc-wrapped, asynchronously locked component.
///
/// Component lifecycle methods are asynchronous, so a Tokio mutex prevents a
/// lifecycle call from blocking an executor thread while it owns mutable
/// component state.
pub type ThreadSafeComponent = Arc<Mutex<BoxedComponent>>;

/// Manages the lifecycle of components
pub struct LifecycleManager {
    components: RwLock<HashMap<String, ThreadSafeComponent>>,
    dependencies: RwLock<DependencyGraph>,
    operation: Mutex<()>,
}

#[derive(Clone, Copy)]
enum LifecycleOperation {
    Init,
    Start,
    Stop,
    Shutdown,
}

impl LifecycleOperation {
    const fn verb(self) -> &'static str {
        match self {
            Self::Init => "initialize",
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Shutdown => "shut down",
        }
    }

    const fn reverse_order(self) -> bool {
        matches!(self, Self::Stop | Self::Shutdown)
    }

    const fn continue_after_failure(self) -> bool {
        matches!(self, Self::Stop | Self::Shutdown)
    }
}

impl LifecycleManager {
    /// Create a new lifecycle manager
    pub fn new() -> Self {
        LifecycleManager {
            components: RwLock::new(HashMap::new()),
            dependencies: RwLock::new(DependencyGraph::new()),
            operation: Mutex::new(()),
        }
    }

    /// Register a component with the lifecycle manager
    pub fn register_component(&self, component: BoxedComponent) -> Result<()> {
        let name = component.name().to_string();
        let dependencies_to_add = component
            .dependencies()
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();

        // Check for existing component with the same name
        let mut components = self.components.write().unwrap();
        if components.contains_key(&name) {
            return Err(LifecycleError::ComponentAlreadyExists(name).into());
        }

        // Stage graph changes so a rejected edge cannot leave a partially
        // registered component behind. Components may be registered before
        // their dependencies; lifecycle execution validates that every graph
        // node has a component before invoking any callback.
        let mut dependencies = self.dependencies.write().unwrap();
        let mut staged_dependencies = dependencies.clone();
        staged_dependencies.add_node(&name);
        for dependency in dependencies_to_add {
            staged_dependencies.add_dependency(&name, &dependency)?;
        }

        *dependencies = staged_dependencies;
        components.insert(name, Arc::new(Mutex::new(component)));

        Ok(())
    }

    /// Initialize all components in dependency order
    pub async fn init_all(&self) -> Result<()> {
        self.run_operation(LifecycleOperation::Init).await
    }

    /// Start all components in dependency order
    pub async fn start_all(&self) -> Result<()> {
        self.run_operation(LifecycleOperation::Start).await
    }

    /// Stop all components in reverse dependency order
    pub async fn stop_all(&self) -> Result<()> {
        self.run_operation(LifecycleOperation::Stop).await
    }

    /// Shut down all components in reverse dependency order
    pub async fn shutdown_all(&self) -> Result<()> {
        self.run_operation(LifecycleOperation::Shutdown).await
    }

    /// Get a component by name
    pub fn get_component(&self, name: &str) -> Option<ThreadSafeComponent> {
        let components = self.components.read().unwrap();
        components.get(name).cloned()
    }

    async fn run_operation(&self, operation: LifecycleOperation) -> Result<()> {
        // Preserve manager-wide lifecycle serialization without holding a
        // blocking registry lock across component futures.
        let _operation = self.operation.lock().await;
        let components = self.component_snapshot(operation.reverse_order())?;
        let mut failures = Vec::new();

        for (name, component) in components {
            let result = {
                let mut component = component.lock().await;
                match operation {
                    LifecycleOperation::Init => component.init().await,
                    LifecycleOperation::Start => component.start().await,
                    LifecycleOperation::Stop => component.stop().await,
                    LifecycleOperation::Shutdown => component.shutdown().await,
                }
            };
            if let Err(error) = result {
                let failure = format!(
                    "Failed to {} component {}: {}",
                    operation.verb(),
                    name,
                    error
                );
                if !operation.continue_after_failure() {
                    return Err(Error::Component(failure));
                }
                failures.push(failure);
            }
        }

        if failures.is_empty() {
            Ok(())
        } else {
            Err(Error::Component(failures.join("; ")))
        }
    }

    /// Take a coherent, immutable view of the registry before awaiting any
    /// component. Components registered after this snapshot participate in
    /// the next lifecycle operation.
    fn component_snapshot(&self, reverse: bool) -> Result<Vec<(String, ThreadSafeComponent)>> {
        // Keep this lock order aligned with register_component.
        let components = self.components.read().unwrap();
        let dependencies = self.dependencies.read().unwrap();
        let mut order = dependencies
            .resolve_order()
            .map_err(|e| Error::Component(format!("Failed to resolve dependencies: {}", e)))?;
        if reverse {
            order.reverse();
        }

        order
            .into_iter()
            .map(|name| {
                let component = components.get(&name).cloned().ok_or_else(|| {
                    let dependent = dependencies
                        .get_dependents(&name)
                        .ok()
                        .and_then(|dependents| dependents.into_iter().min())
                        .unwrap_or_else(|| "registered component".to_owned());
                    Error::from(LifecycleError::DependencyNotFound(name.clone(), dependent))
                })?;
                Ok((name, component))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::Mutex as StdMutex;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum FailurePoint {
        Start,
        Stop,
        Shutdown,
    }

    struct RecordingComponent {
        name: &'static str,
        dependencies: Vec<&'static str>,
        events: Arc<StdMutex<Vec<String>>>,
        failure: Option<FailurePoint>,
    }

    impl RecordingComponent {
        fn new(
            name: &'static str,
            dependencies: Vec<&'static str>,
            events: Arc<StdMutex<Vec<String>>>,
        ) -> Self {
            Self {
                name,
                dependencies,
                events,
                failure: None,
            }
        }

        fn failing(mut self, failure: FailurePoint) -> Self {
            self.failure = Some(failure);
            self
        }

        fn record(&self, operation: &str) {
            self.events
                .lock()
                .unwrap()
                .push(format!("{operation}:{}", self.name));
        }

        fn fail(&self, point: FailurePoint) -> Result<()> {
            if self.failure == Some(point) {
                Err(Error::Component(format!("{} failure", self.name)))
            } else {
                Ok(())
            }
        }
    }

    #[async_trait]
    impl Component for RecordingComponent {
        fn name(&self) -> &str {
            self.name
        }

        fn state(&self) -> ComponentState {
            ComponentState::Created
        }

        async fn init(&mut self) -> Result<()> {
            self.record("init");
            Ok(())
        }

        async fn start(&mut self) -> Result<()> {
            self.record("start");
            self.fail(FailurePoint::Start)
        }

        async fn stop(&mut self) -> Result<()> {
            self.record("stop");
            self.fail(FailurePoint::Stop)
        }

        async fn shutdown(&mut self) -> Result<()> {
            self.record("shutdown");
            self.fail(FailurePoint::Shutdown)
        }

        fn dependencies(&self) -> Vec<&str> {
            self.dependencies.clone()
        }
    }

    fn position(events: &[String], expected: &str) -> usize {
        events.iter().position(|event| event == expected).unwrap()
    }

    #[tokio::test]
    async fn lifecycle_uses_dependency_order_and_includes_independent_components() {
        let manager = LifecycleManager::new();
        let events = Arc::new(StdMutex::new(Vec::new()));
        manager
            .register_component(Box::new(RecordingComponent::new(
                "api",
                vec!["database"],
                events.clone(),
            )))
            .unwrap();
        manager
            .register_component(Box::new(RecordingComponent::new(
                "metrics",
                vec![],
                events.clone(),
            )))
            .unwrap();
        manager
            .register_component(Box::new(RecordingComponent::new(
                "database",
                vec![],
                events.clone(),
            )))
            .unwrap();

        manager.init_all().await.unwrap();
        manager.start_all().await.unwrap();
        manager.stop_all().await.unwrap();
        manager.shutdown_all().await.unwrap();

        let events = events.lock().unwrap();
        assert!(position(&events, "init:database") < position(&events, "init:api"));
        assert!(position(&events, "start:database") < position(&events, "start:api"));
        assert!(position(&events, "stop:api") < position(&events, "stop:database"));
        assert!(position(&events, "shutdown:api") < position(&events, "shutdown:database"));
        for operation in ["init", "start", "stop", "shutdown"] {
            assert!(events.contains(&format!("{operation}:metrics")));
        }
    }

    #[tokio::test]
    async fn missing_dependency_fails_before_any_component_callback() {
        let manager = LifecycleManager::new();
        let events = Arc::new(StdMutex::new(Vec::new()));
        manager
            .register_component(Box::new(RecordingComponent::new(
                "api",
                vec!["missing"],
                events.clone(),
            )))
            .unwrap();

        let error = manager.init_all().await.unwrap_err().to_string();
        assert!(error.contains("Dependency not found: missing required by api"));
        assert!(events.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn startup_is_fail_fast_but_teardown_continues_through_failures() {
        let manager = LifecycleManager::new();
        let events = Arc::new(StdMutex::new(Vec::new()));
        manager
            .register_component(Box::new(
                RecordingComponent::new("database", vec![], events.clone())
                    .failing(FailurePoint::Start),
            ))
            .unwrap();
        manager
            .register_component(Box::new(
                RecordingComponent::new("api", vec!["database"], events.clone())
                    .failing(FailurePoint::Stop),
            ))
            .unwrap();

        assert!(manager.start_all().await.is_err());
        assert_eq!(
            events.lock().unwrap().as_slice(),
            ["start:database".to_owned()]
        );

        events.lock().unwrap().clear();
        let error = manager.stop_all().await.unwrap_err().to_string();
        assert!(error.contains("Failed to stop component api"));
        assert_eq!(
            events.lock().unwrap().as_slice(),
            ["stop:api".to_owned(), "stop:database".to_owned()]
        );
    }

    #[tokio::test]
    async fn shutdown_continues_to_dependencies_after_a_dependent_fails() {
        let manager = LifecycleManager::new();
        let events = Arc::new(StdMutex::new(Vec::new()));
        manager
            .register_component(Box::new(RecordingComponent::new(
                "database",
                vec![],
                events.clone(),
            )))
            .unwrap();
        manager
            .register_component(Box::new(
                RecordingComponent::new("api", vec!["database"], events.clone())
                    .failing(FailurePoint::Shutdown),
            ))
            .unwrap();

        assert!(manager.shutdown_all().await.is_err());
        assert_eq!(
            events.lock().unwrap().as_slice(),
            ["shutdown:api".to_owned(), "shutdown:database".to_owned()]
        );
    }
}

impl Debug for LifecycleManager {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let components = self.components.read().unwrap();
        let component_count = components.len();

        f.debug_struct("LifecycleManager")
            .field("component_count", &component_count)
            .finish()
    }
}

impl Default for LifecycleManager {
    fn default() -> Self {
        Self::new()
    }
}
