use crate::errors::types::{Error, Result};
use std::collections::{HashMap, HashSet, VecDeque};
use thiserror::Error;

/// Errors related to dependency resolution
#[derive(Error, Debug)]
pub enum DependencyError {
    #[error("Circular dependency detected: {0}")]
    CircularDependency(String),
    
    #[error("Dependency not found: {0}")]
    DependencyNotFound(String),
    
    #[error("Node not found: {0}")]
    NodeNotFound(String),
}

impl From<DependencyError> for Error {
    fn from(err: DependencyError) -> Self {
        Error::Dependency(err.to_string())
    }
}

/// A graph structure that manages dependencies between components
#[derive(Debug, Default)]
pub struct DependencyGraph {
    /// Map of component name to its dependencies
    dependencies: HashMap<String, HashSet<String>>,
    /// Map of component name to components that depend on it (reverse dependencies)
    reverse_dependencies: HashMap<String, HashSet<String>>,
}

impl DependencyGraph {
    /// Create a new empty dependency graph
    pub fn new() -> Self {
        DependencyGraph {
            dependencies: HashMap::new(),
            reverse_dependencies: HashMap::new(),
        }
    }
    
    /// Add a component to the graph without any dependencies
    pub fn add_node(&mut self, name: &str) {
        self.dependencies.entry(name.to_string()).or_insert_with(HashSet::new);
        self.reverse_dependencies.entry(name.to_string()).or_insert_with(HashSet::new);
    }
    
    /// Add a dependency relationship between components
    pub fn add_dependency(&mut self, dependent: &str, dependency: &str) -> Result<()> {
        // Ensure both nodes exist
        self.dependencies.entry(dependent.to_string()).or_insert_with(HashSet::new);
        self.reverse_dependencies.entry(dependent.to_string()).or_insert_with(HashSet::new);
        
        self.dependencies.entry(dependency.to_string()).or_insert_with(HashSet::new);
        self.reverse_dependencies.entry(dependency.to_string()).or_insert_with(HashSet::new);
        
        // Add the dependency relationship
        self.dependencies.get_mut(dependent).unwrap().insert(dependency.to_string());
        self.reverse_dependencies.get_mut(dependency).unwrap().insert(dependent.to_string());
        
        // Check for circular dependencies
        if self.has_circular_dependencies() {
            // Remove the dependency we just added
            self.dependencies.get_mut(dependent).unwrap().remove(dependency);
            self.reverse_dependencies.get_mut(dependency).unwrap().remove(dependent);
            
            return Err(DependencyError::CircularDependency(
                format!("{} -> {}", dependent, dependency)
            ).into());
        }
        
        Ok(())
    }
    
    /// Remove a dependency relationship
    pub fn remove_dependency(&mut self, dependent: &str, dependency: &str) -> Result<()> {
        if !self.dependencies.contains_key(dependent) {
            return Err(DependencyError::NodeNotFound(dependent.to_string()).into());
        }
        
        if !self.dependencies.contains_key(dependency) {
            return Err(DependencyError::NodeNotFound(dependency.to_string()).into());
        }
        
        self.dependencies.get_mut(dependent).unwrap().remove(dependency);
        self.reverse_dependencies.get_mut(dependency).unwrap().remove(dependent);
        
        Ok(())
    }
    
    /// Get the direct dependencies of a component
    pub fn get_dependencies(&self, name: &str) -> Result<HashSet<String>> {
        self.dependencies.get(name)
            .cloned()
            .ok_or_else(|| DependencyError::NodeNotFound(name.to_string()).into())
    }
    
    /// Get components that directly depend on the given component
    pub fn get_dependents(&self, name: &str) -> Result<HashSet<String>> {
        self.reverse_dependencies.get(name)
            .cloned()
            .ok_or_else(|| DependencyError::NodeNotFound(name.to_string()).into())
    }
    
    /// Check if the graph has any circular dependencies
    pub fn has_circular_dependencies(&self) -> bool {
        for node in self.dependencies.keys() {
            let mut visited = HashSet::new();
            let mut path = HashSet::new();
            
            if self.is_cyclic_util(node, &mut visited, &mut path) {
                return true;
            }
        }
        
        false
    }
    
    /// Helper function for cycle detection
    fn is_cyclic_util(&self, node: &str, visited: &mut HashSet<String>, path: &mut HashSet<String>) -> bool {
        if !visited.contains(node) {
            visited.insert(node.to_string());
            path.insert(node.to_string());
            
            if let Some(deps) = self.dependencies.get(node) {
                for dep in deps {
                    if !visited.contains(dep) && self.is_cyclic_util(dep, visited, path) {
                        return true;
                    } else if path.contains(dep) {
                        return true;
                    }
                }
            }
        }
        
        path.remove(node);
        false
    }
    
    /// Resolve the initialization order using topological sort
    pub fn resolve_order(&self) -> Result<Vec<String>> {
        let mut result = Vec::new();
        let mut in_degree = HashMap::new();
        let mut queue = VecDeque::new();
        
        // Initialize in-degree for all nodes
        for node in self.dependencies.keys() {
            in_degree.insert(node.clone(), 0);
        }
        
        // Calculate in-degree for each node
        for (_, deps) in &self.dependencies {
            for dep in deps {
                *in_degree.entry(dep.clone()).or_insert(0) += 1;
            }
        }
        
        // Add all nodes with in-degree 0 to the queue
        for (node, degree) in &in_degree {
            if *degree == 0 {
                queue.push_back(node.clone());
            }
        }
        
        // Process queue
        while let Some(node) = queue.pop_front() {
            result.push(node.clone());
            
            if let Some(dependents) = self.reverse_dependencies.get(&node) {
                for dependent in dependents {
                    let degree = in_degree.get_mut(dependent).unwrap();
                    *degree -= 1;
                    
                    if *degree == 0 {
                        queue.push_back(dependent.clone());
                    }
                }
            }
        }
        
        // Check if we resolved all nodes
        if result.len() != self.dependencies.len() {
            return Err(DependencyError::CircularDependency("Cycle detected during topological sort".to_string()).into());
        }
        
        Ok(result)
    }
} 