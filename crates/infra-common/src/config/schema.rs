use crate::errors::types::{Error, Result};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::fmt::Debug;
use std::marker::PhantomData;

/// Validates configuration against a schema
#[derive(Debug)]
pub struct SchemaValidator<T> {
    schema_name: String,
    _phantom: PhantomData<T>,
}

impl<T: DeserializeOwned + Debug + 'static> SchemaValidator<T> {
    /// Create a new schema validator for a specific type
    pub fn new(schema_name: &str) -> Self {
        SchemaValidator {
            schema_name: schema_name.to_string(),
            _phantom: PhantomData,
        }
    }
    
    /// Validate a JSON value against this schema
    pub fn validate(&self, value: &Value) -> Result<T> {
        // In a real implementation, this would use JSON Schema validation
        // For the stub, we'll just try to deserialize the value
        serde_json::from_value(value.clone())
            .map_err(|e| Error::Validation(format!("Schema validation failed for {}: {}", self.schema_name, e)))
    }
    
    /// Validate a configuration string against this schema
    pub fn validate_str(&self, json_str: &str) -> Result<T> {
        let value: Value = serde_json::from_str(json_str)
            .map_err(|e| Error::Validation(format!("Invalid JSON: {}", e)))?;
        
        self.validate(&value)
    }
}

/// Helper function to create and use a schema validator
pub fn validate_config<T: DeserializeOwned + Debug + 'static>(
    schema_name: &str,
    config_str: &str,
) -> Result<T> {
    SchemaValidator::<T>::new(schema_name).validate_str(config_str)
}

/// Trait for configuration types that can validate themselves
pub trait SelfValidating: Sized {
    /// Validate the configuration
    fn validate(&self) -> Result<()>;
    
    /// Validate after loading
    fn validate_after_load(self) -> Result<Self> {
        self.validate()?;
        Ok(self)
    }
} 