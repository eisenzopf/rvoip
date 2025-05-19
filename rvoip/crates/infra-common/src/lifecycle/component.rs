use async_trait::async_trait;
use std::fmt::Debug;
use crate::errors::types::Error;

/// Possible states of a component in its lifecycle
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentState {
    /// Component has been created but not initialized
    Created,
    /// Component has been initialized but not started
    Initialized,
    /// Component is running
    Running,
    /// Component has been paused
    Paused,
    /// Component has been stopped
    Stopped,
    /// Component has been shut down
    ShutDown,
    /// Component is in an error state
    Error,
}

/// Trait that defines the standard lifecycle for all components
#[async_trait]
pub trait Component: Send + Sync {
    /// Get the unique name of this component
    fn name(&self) -> &str;
    
    /// Get the current state of this component
    fn state(&self) -> ComponentState;
    
    /// Initialize the component
    /// 
    /// This is called once when the component is first created.
    /// Components should perform any setup that doesn't require
    /// other components to be running.
    async fn init(&mut self) -> Result<(), Error>;
    
    /// Start the component
    /// 
    /// This is called after initialization and should start any
    /// background tasks or services provided by the component.
    async fn start(&mut self) -> Result<(), Error>;
    
    /// Pause the component
    /// 
    /// This is an optional state that components can implement
    /// to temporarily suspend processing without fully stopping.
    async fn pause(&mut self) -> Result<(), Error> {
        // Default implementation does nothing
        Ok(())
    }
    
    /// Resume the component from a paused state
    async fn resume(&mut self) -> Result<(), Error> {
        // Default implementation does nothing
        Ok(())
    }
    
    /// Stop the component
    /// 
    /// This should stop any background tasks or services but
    /// preserve state so the component can be restarted.
    async fn stop(&mut self) -> Result<(), Error>;
    
    /// Shut down the component
    /// 
    /// This is called when the component is being permanently
    /// removed and should release all resources.
    async fn shutdown(&mut self) -> Result<(), Error>;
    
    /// Get a list of component names that this component depends on
    fn dependencies(&self) -> Vec<&str> {
        // Default implementation has no dependencies
        vec![]
    }
    
    /// Check health status of the component
    async fn health_check(&self) -> Result<(), Error> {
        // Default implementation reports healthy if in Running state
        match self.state() {
            ComponentState::Running => Ok(()),
            _ => Err(Error::ComponentNotReady(self.name().to_string())),
        }
    }
} 