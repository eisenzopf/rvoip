use std::sync::Arc;
use std::collections::HashMap;
use std::net::SocketAddr;

use anyhow::Result;
use tracing::{debug, info, warn, error};
use serde::{Serialize, Deserialize};

use rvoip_sip_core::{Request, Response, Method, StatusCode, Uri};

use crate::errors::Error;
use crate::registry::{Registry, Registration};

/// Routing priority
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RoutingPriority {
    /// Highest priority route
    Primary = 0,
    /// Secondary route
    Secondary = 1,
    /// Tertiary route
    Tertiary = 2,
    /// Fall back route (lowest priority)
    Fallback = 3,
}

impl Default for RoutingPriority {
    fn default() -> Self {
        Self::Primary
    }
}

/// A route for a call
#[derive(Debug, Clone)]
pub struct Route {
    /// Destination URI
    pub target: Uri,
    
    /// Network address of the target
    pub address: Option<SocketAddr>,
    
    /// Priority of this route
    pub priority: RoutingPriority,
    
    /// Whether this route is accessible directly
    pub is_direct: bool,
    
    /// Associated registration (if any)
    pub registration: Option<Registration>,
}

impl Route {
    /// Create a new route from a registration
    pub fn from_registration(registration: Registration) -> Self {
        Self {
            target: registration.contact.clone(),
            address: Some(registration.address),
            priority: RoutingPriority::Primary,
            is_direct: true,
            registration: Some(registration),
        }
    }
    
    /// Create a new route to a URI with a specific address
    pub fn new(uri: Uri, address: SocketAddr) -> Self {
        Self {
            target: uri,
            address: Some(address),
            priority: RoutingPriority::Primary,
            is_direct: true,
            registration: None,
        }
    }
    
    /// Create a new route to an external target
    pub fn external(uri: Uri) -> Self {
        Self {
            target: uri,
            address: None,
            priority: RoutingPriority::Primary,
            is_direct: false,
            registration: None,
        }
    }
}

/// A router for SIP calls
pub struct Router {
    /// Registry for lookups
    registry: Arc<Registry>,
    
    /// Static routes (URI pattern -> target URIs)
    static_routes: HashMap<String, Vec<Route>>,
    
    /// Domain routes (domain -> target)
    domain_routes: HashMap<String, Vec<Route>>,
}

impl Router {
    /// Create a new router
    pub fn new(registry: Arc<Registry>) -> Self {
        Self {
            registry,
            static_routes: HashMap::new(),
            domain_routes: HashMap::new(),
        }
    }
    
    /// Add a static route
    pub fn add_static_route(&mut self, pattern: String, route: Route) {
        self.static_routes.entry(pattern)
            .or_insert_with(Vec::new)
            .push(route);
    }
    
    /// Add a domain route
    pub fn add_domain_route(&mut self, domain: String, route: Route) {
        self.domain_routes.entry(domain)
            .or_insert_with(Vec::new)
            .push(route);
    }
    
    /// Find routes for a request URI
    pub fn find_routes(&self, uri: &Uri) -> Result<Vec<Route>, Error> {
        let mut routes = Vec::new();
        
        // Check for registration-based routing
        if let Some(registration) = self.registry.lookup(uri) {
            routes.push(Route::from_registration(registration));
            return Ok(routes);
        }
        
        // Check static routes
        let uri_str = uri.to_string();
        for (pattern, pattern_routes) in &self.static_routes {
            if uri_str.contains(pattern) {
                routes.extend(pattern_routes.clone());
            }
        }
        
        // Check domain routes if we have a host
        let host = &uri.host;
        let host_str = host.to_string();
        if let Some(domain_routes) = self.domain_routes.get(&host_str) {
            routes.extend(domain_routes.clone());
        }
        
        if routes.is_empty() {
            return Err(Error::routing(format!("No route found for URI: {}", uri)));
        }
        
        // Sort routes by priority
        routes.sort_by_key(|r| r.priority);
        
        Ok(routes)
    }
    
    /// Route a SIP request
    pub fn route_request(&self, request: &Request) -> Result<Vec<Route>, Error> {
        // Get the destination URI
        let request_uri = &request.uri;
        
        // For REGISTER requests, route to local registrar
        if request.method == Method::Register {
            // Local registration handling
            return Err(Error::routing("REGISTER requests should be handled locally"));
        }
        
        self.find_routes(request_uri)
    }
    
    /// Prepare a request for forwarding to a route
    pub fn prepare_request_for_route(&self, request: &mut Request, route: &Route) {
        // Update the request URI
        request.uri = route.target.clone();
        
        // Add/update Via header
        // TODO: Implement Via header manipulation
        
        // Add/update Route header if needed
        // TODO: Implement Route header manipulation
        
        // Add Record-Route if acting as proxy
        // TODO: Implement Record-Route
    }
}

/// A route target specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteTarget {
    /// URI pattern to match
    pub pattern: String,
    
    /// Target URI
    pub target: String,
    
    /// Priority
    pub priority: RoutingPriority,
    
    /// Target address (if direct)
    pub address: Option<String>,
}

/// Router configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouterConfig {
    /// Static routes
    pub static_routes: Vec<RouteTarget>,
    
    /// Domain routes
    pub domain_routes: HashMap<String, Vec<RouteTarget>>,
}

impl RouterConfig {
    /// Apply this configuration to a router
    pub fn apply_to_router(&self, router: &mut Router) -> Result<(), Error> {
        // Clear existing configuration
        router.static_routes.clear();
        router.domain_routes.clear();
        
        // Apply static routes
        for route_target in &self.static_routes {
            let target_uri = route_target.target.parse::<Uri>()
                .map_err(|_| Error::routing(format!("Invalid target URI: {}", route_target.target)))?;
            
            let route = if let Some(addr_str) = &route_target.address {
                let addr = addr_str.parse::<SocketAddr>()
                    .map_err(|_| Error::routing(format!("Invalid address: {}", addr_str)))?;
                Route::new(target_uri, addr)
            } else {
                Route::external(target_uri)
            };
            
            router.add_static_route(route_target.pattern.clone(), route);
        }
        
        // Apply domain routes
        for (domain, targets) in &self.domain_routes {
            for route_target in targets {
                let target_uri = route_target.target.parse::<Uri>()
                    .map_err(|_| Error::routing(format!("Invalid target URI: {}", route_target.target)))?;
                
                let route = if let Some(addr_str) = &route_target.address {
                    let addr = addr_str.parse::<SocketAddr>()
                        .map_err(|_| Error::routing(format!("Invalid address: {}", addr_str)))?;
                    Route::new(target_uri, addr)
                } else {
                    Route::external(target_uri)
                };
                
                router.add_domain_route(domain.clone(), route);
            }
        }
        
        Ok(())
    }
} 