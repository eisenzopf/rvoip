---
section: Breaking changes
breaking: true
---
**quick-xml 0.41 → 0.42 in `rvoip-sip-registrar`.** `RegistrarError`
implements `From<quick_xml::Error>` for quick-xml 0.42. Migration: move
your own quick-xml dependency to 0.42 if you rely on that conversion.
