use rvoip_sip_core::prelude::*;
use rvoip_sip_core::TypedHeader;
use rvoip_sip_core::builder::headers::ViaBuilderExt;

use crate::error::Result;

/// ResponseBuilderExt trait - Use specific accessors and wrap headers
pub trait ResponseBuilderExt {
    fn copy_essential_headers(self, request: &Request) -> Result<Self> where Self: Sized;
}

impl ResponseBuilderExt for ResponseBuilder {
    fn copy_essential_headers(mut self, request: &Request) -> Result<Self> {
        if let Some(via) = request.first_via() {
            self = self.header(TypedHeader::Via(via.clone()));
        }
        if let Some(to) = request.header(&HeaderName::To) {
            if let TypedHeader::To(to_val) = to {
                self = self.header(TypedHeader::To(to_val.clone()));
            }
        }
        if let Some(from) = request.header(&HeaderName::From) {
            if let TypedHeader::From(from_val) = from {
                self = self.header(TypedHeader::From(from_val.clone()));
            }
        }
        if let Some(call_id) = request.header(&HeaderName::CallId) {
            if let TypedHeader::CallId(call_id_val) = call_id {
                self = self.header(TypedHeader::CallId(call_id_val.clone()));
            }
        }
        if let Some(cseq) = request.header(&HeaderName::CSeq) {
            if let TypedHeader::CSeq(cseq_val) = cseq {
                self = self.header(TypedHeader::CSeq(cseq_val.clone()));
            }
        }
        self = self.header(TypedHeader::ContentLength(ContentLength::new(0)));
        Ok(self)
    }
} 