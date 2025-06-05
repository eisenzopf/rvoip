/// Add business hours filtering
pub fn with_business_hours(mut self, start: &str, end: &str, timezone: &str) -> Self {
    let handler = BusinessHoursHandler::new(start, end, timezone)
        .with_name(format!("{}_BusinessHours", self.name));
    self.handlers.push((handler, 800));
    self
}

/// Add whitelist filtering
pub fn with_whitelist(mut self, allowed_callers: Vec<String>) -> Self {
    let handler = WhitelistHandler::new(&format!("{}_Whitelist", self.name))
        .with_allowed_callers(allowed_callers);
    self.handlers.push((handler, 900));
    self
} 