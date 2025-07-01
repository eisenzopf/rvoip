use crate::lifecycle::component::{Component, ComponentState};
use crate::lifecycle::dependency::DependencyGraph;
use crate::errors::types::{Error, Result};
use std::collections::HashMap;
use std::fmt::{self, Debug, Formatter};
use std::sync::{Arc, RwLock, Mutex};
use thiserror::Error;


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

/// Type alias for an Arc-wrapped, Mutex-protected component
pub type ThreadSafeComponent = Arc<Mutex<BoxedComponent>>;

/// Manages the lifecycle of components
pub struct LifecycleManager {
    components: RwLock<HashMap<String, ThreadSafeComponent>>,
    dependencies: RwLock<DependencyGraph>,
}

impl LifecycleManager {
    /// Create a new lifecycle manager
    pub fn new() -> Self {
        LifecycleManager {
            components: RwLock::new(HashMap::new()),
            dependencies: RwLock::new(DependencyGraph::new()),
        }
    }
    
    /// Register a component with the lifecycle manager
    pub fn register_component(&self, component: BoxedComponent) -> Result<()> {
        let name = component.name().to_string();
        let deps = component.dependencies();
        
        // Check for existing component with the same name
        let mut components = self.components.write().unwrap();
        if components.contains_key(&name) {
            return Err(LifecycleError::ComponentAlreadyExists(name).into());
        }
        
        // Register dependencies
        let mut dependencies = self.dependencies.write().unwrap();
        for dep in deps {
            dependencies.add_dependency(&name, dep)?;
        }
        
        // Register the component - wrap in Arc<Mutex<>> for mutable sharing
        components.insert(name, Arc::new(Mutex::new(component)));
        
        Ok(())
    }
    
    /// Initialize all components in dependency order
    pub async fn init_all(&self) -> Result<()> {
        let mut components = self.components.write().unwrap();
        let order = self.get_init_order()?;
        
        for name in order {
            if let Some(component) = components.get(&name) {
                // Mutex to access mutably
                let mut component_guard = component.lock().unwrap();
                if let Err(e) = component_guard.init().await {
                    return Err(Error::Component(
                        format!("Failed to initialize component {}: {}", name, e)
                    ));
                }
            }
        }
        
        Ok(())
    }
    
    /// Start all components in dependency order
    pub async fn start_all(&self) -> Result<()> {
        let mut components = self.components.write().unwrap();
        let order = self.get_init_order()?;
        
        for name in order {
            if let Some(component) = components.get(&name) {
                // Mutex to access mutably
                let mut component_guard = component.lock().unwrap();
                if let Err(e) = component_guard.start().await {
                    return Err(Error::Component(
                        format!("Failed to start component {}: {}", name, e)
                    ));
                }
            }
        }
        
        Ok(())
    }
    
    /// Stop all components in reverse dependency order
    pub async fn stop_all(&self) -> Result<()> {
        let mut components = self.components.write().unwrap();
        let mut order = self.get_init_order()?;
        order.reverse(); // Reverse order for shutdown
        
        for name in order {
            if let Some(component) = components.get(&name) {
                // Mutex to access mutably
                let mut component_guard = component.lock().unwrap();
                if let Err(e) = component_guard.stop().await {
                    return Err(Error::Component(
                        format!("Failed to stop component {}: {}", name, e)
                    ));
                }
            }
        }
        
        Ok(())
    }
    
    /// Shut down all components in reverse dependency order
    pub async fn shutdown_all(&self) -> Result<()> {
        let mut components = self.components.write().unwrap();
        let mut order = self.get_init_order()?;
        order.reverse(); // Reverse order for shutdown
        
        for name in order {
            if let Some(component) = components.get(&name) {
                // Mutex to access mutably
                let mut component_guard = component.lock().unwrap();
                if let Err(e) = component_guard.shutdown().await {
                    return Err(Error::Component(
                        format!("Failed to shut down component {}: {}", name, e)
                    ));
                }
            }
        }
        
        Ok(())
    }
    
    /// Get a component by name
    pub fn get_component(&self, name: &str) -> Option<ThreadSafeComponent> {
        let components = self.components.read().unwrap();
        components.get(name).cloned()
    }
    
    /// Get the initialization order based on dependencies
    fn get_init_order(&self) -> Result<Vec<String>> {
        let dependencies = self.dependencies.read().unwrap();
        let order = dependencies.resolve_order()
            .map_err(|e| Error::Component(format!("Failed to resolve dependencies: {}", e)))?;
        
        Ok(order)
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