//! Security headers middleware

use axum::{
    middleware::Next,
    response::Response,
    http::{Request, header, HeaderValue},
};

/// Add security headers to all responses
pub async fn security_headers_middleware(
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    
    // Enforce HTTPS (HTTP Strict Transport Security)
    // max-age=31536000 (1 year), include subdomains
    headers.insert(
        header::STRICT_TRANSPORT_SECURITY,
        HeaderValue::from_static("max-age=31536000; includeSubDomains")
    );
    
    // Prevent clickjacking attacks
    headers.insert(
        header::X_FRAME_OPTIONS,
        HeaderValue::from_static("DENY")
    );
    
    // Prevent MIME type sniffing
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff")
    );
    
    // Enable XSS protection (for older browsers)
    headers.insert(
        "X-XSS-Protection",
        HeaderValue::from_static("1; mode=block")
    );
    
    // Control referrer information
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("strict-origin-when-cross-origin")
    );
    
    // Content Security Policy
    // This is a restrictive policy suitable for an API
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'none'; \
             frame-ancestors 'none'; \
             form-action 'none'; \
             base-uri 'none'; \
             upgrade-insecure-requests"
        )
    );
    
    // Permissions Policy (formerly Feature Policy)
    headers.insert(
        "Permissions-Policy",
        HeaderValue::from_static(
            "accelerometer=(), \
             camera=(), \
             geolocation=(), \
             gyroscope=(), \
             magnetometer=(), \
             microphone=(), \
             payment=(), \
             usb=()"
        )
    );
    
    response
}
