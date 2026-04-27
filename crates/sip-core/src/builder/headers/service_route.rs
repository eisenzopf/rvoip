use super::HeaderSetter;
use crate::error::{Error, Result};
use crate::types::{service_route::ServiceRoute, uri::Uri};
use std::str::FromStr;

/// Service-Route header builder (RFC 3608).
///
/// Service-Route is returned by a registrar in a 2xx REGISTER response to give
/// the UA a pre-loaded Route set for subsequent out-of-dialog requests. When a
/// registered UA originates an out-of-dialog request within the same
/// registration binding, it MUST pre-load the stored URIs as Route headers in
/// the order received. This is the outbound-path counterpart to Path (RFC 3327).
pub trait ServiceRouteBuilderExt {
    /// Add a Service-Route header with a single URI.
    fn service_route(self, uri: impl AsRef<str>) -> Result<Self>
    where
        Self: Sized;

    /// Add a Service-Route header with multiple URIs, in order.
    fn service_route_uris(self, uris: Vec<impl AsRef<str>>) -> Result<Self>
    where
        Self: Sized;
}

impl<T> ServiceRouteBuilderExt for T
where
    T: HeaderSetter,
{
    fn service_route(self, uri: impl AsRef<str>) -> Result<Self> {
        let uri = Uri::from_str(uri.as_ref())?;
        Ok(self.set_header(ServiceRoute::with_uri(uri)))
    }

    fn service_route_uris(self, uris: Vec<impl AsRef<str>>) -> Result<Self> {
        let mut sr = ServiceRoute::empty();
        for u in uris {
            let uri = Uri::from_str(u.as_ref()).map_err(Error::from)?;
            sr.add_uri(uri);
        }
        Ok(self.set_header(sr))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{headers::HeaderName, method::Method, StatusCode, TypedHeader};
    use crate::{RequestBuilder, ResponseBuilder};

    #[test]
    fn test_response_service_route_single() {
        let response = ResponseBuilder::new(StatusCode::Ok, None)
            .service_route("sip:orig-proxy.example.com;lr")
            .unwrap()
            .build();

        if let Some(TypedHeader::ServiceRoute(sr)) = response.header(&HeaderName::ServiceRoute) {
            assert_eq!(sr.len(), 1);
            assert_eq!(sr[0].0.uri.to_string(), "sip:orig-proxy.example.com;lr");
        } else {
            panic!("Service-Route header not found");
        }
    }

    #[test]
    fn test_response_service_route_multiple() {
        let response = ResponseBuilder::new(StatusCode::Ok, None)
            .service_route_uris(vec!["sip:p1.example.com;lr", "sip:p2.example.com;lr"])
            .unwrap()
            .build();

        if let Some(TypedHeader::ServiceRoute(sr)) = response.header(&HeaderName::ServiceRoute) {
            assert_eq!(sr.len(), 2);
            assert_eq!(sr[0].0.uri.to_string(), "sip:p1.example.com;lr");
            assert_eq!(sr[1].0.uri.to_string(), "sip:p2.example.com;lr");
        } else {
            panic!("Service-Route header not found");
        }
    }

    #[test]
    fn test_request_service_route_invalid_uri() {
        let err = RequestBuilder::new(Method::Register, "sip:registrar.example.com")
            .unwrap()
            .service_route("not a uri");
        assert!(err.is_err());
    }
}
