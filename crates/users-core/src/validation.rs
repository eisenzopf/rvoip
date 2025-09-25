//! Input validation module with security-focused validation rules

use validator::{Validate, ValidationError};
use regex::Regex;
use once_cell::sync::Lazy;
use std::collections::HashSet;

// Regex patterns for validation
static USERNAME_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[a-zA-Z0-9_.-]{3,32}$").unwrap());
static EMAIL_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[^\s@]+@[^\s@]+\.[^\s@]+$").unwrap());

// Common passwords list (top 100 for demo - in production, use larger list)
static COMMON_PASSWORDS: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    vec![
        "password", "123456", "password123", "admin", "letmein", "welcome", "monkey",
        "dragon", "baseball", "iloveyou", "trustno1", "1234567", "sunshine", "master",
        "123456789", "welcome123", "shadow", "ashley", "football", "jesus", "michael",
        "ninja", "mustang", "password1", "123123", "pass", "12345678", "abc123",
        "qwerty", "111111", "qwertyuiop", "1234567890", "password1234", "changeme",
    ].into_iter().collect()
});

/// Password policy configuration
#[derive(Debug, Clone)]
pub struct PasswordPolicy {
    pub min_length: usize,
    pub max_length: usize,
    pub require_uppercase: bool,
    pub require_lowercase: bool,
    pub require_numbers: bool,
    pub require_special: bool,
    pub min_unique_chars: usize,
    pub disallow_common_passwords: bool,
    pub disallow_username_in_password: bool,
    pub max_consecutive_chars: usize,
}

impl Default for PasswordPolicy {
    fn default() -> Self {
        Self {
            min_length: 12,
            max_length: 128,
            require_uppercase: true,
            require_lowercase: true,
            require_numbers: true,
            require_special: false,  // Optional as recommended
            min_unique_chars: 6,
            disallow_common_passwords: true,
            disallow_username_in_password: true,
            max_consecutive_chars: 3,
        }
    }
}

/// Validated create user request
#[derive(Debug, Validate)]
pub struct ValidatedCreateUserRequest {
    #[validate(length(min = 3, max = 32), custom(function = "validate_username_format"))]
    pub username: String,
    
    #[validate(length(min = 8, max = 128))]
    pub password: String,
    
    #[validate(email)]
    pub email: Option<String>,
    
    #[validate(length(max = 100))]
    pub display_name: Option<String>,
    
    #[validate(custom(function = "validate_roles"))]
    pub roles: Vec<String>,
}

/// Validate username format
fn validate_username_format(username: &str) -> Result<(), ValidationError> {
    if USERNAME_REGEX.is_match(username) {
        Ok(())
    } else {
        Err(ValidationError::new("invalid_username_format"))
    }
}

/// Validate user roles against whitelist
pub fn validate_roles(roles: &Vec<String>) -> Result<(), ValidationError> {
    const ALLOWED_ROLES: &[&str] = &["user", "admin", "moderator", "guest"];
    
    for role in roles {
        if !ALLOWED_ROLES.contains(&role.as_str()) {
            return Err(ValidationError::new("invalid_role"));
        }
    }
    
    if roles.len() > 10 {
        return Err(ValidationError::new("too_many_roles"));
    }
    
    Ok(())
}

/// Sanitize display name to prevent XSS
pub fn sanitize_display_name(name: &str) -> String {
    // Remove any HTML/script tags
    let re = Regex::new(r"<[^>]+>").unwrap();
    let sanitized = re.replace_all(name, "").to_string();
    
    // Trim to max length
    if sanitized.len() > 100 {
        sanitized.chars().take(100).collect()
    } else {
        sanitized
    }
}

/// Validate search input to prevent SQL injection
pub fn validate_search_input(search: &str) -> Result<String, ValidationError> {
    if search.len() > 100 {
        return Err(ValidationError::new("search_too_long"));
    }
    
    // We don't need to check for SQL patterns since we're using parameterized queries
    // But we can still do basic validation
    if search.is_empty() {
        return Err(ValidationError::new("search_empty"));
    }
    
    Ok(search.to_string())
}

/// Password validator
pub struct PasswordValidator {
    policy: PasswordPolicy,
}

impl PasswordValidator {
    pub fn new(policy: PasswordPolicy) -> Self {
        Self { policy }
    }
    
    pub fn with_default_policy() -> Self {
        Self::new(PasswordPolicy::default())
    }
    
    pub fn validate(&self, password: &str, username: &str) -> Result<(), PasswordError> {
        // Length checks
        if password.len() < self.policy.min_length {
            return Err(PasswordError::TooShort(self.policy.min_length));
        }
        
        if password.len() > self.policy.max_length {
            return Err(PasswordError::TooLong(self.policy.max_length));
        }
        
        // Character class requirements
        let has_upper = password.chars().any(|c| c.is_uppercase());
        let has_lower = password.chars().any(|c| c.is_lowercase());
        let has_digit = password.chars().any(|c| c.is_numeric());
        let has_special = password.chars().any(|c| !c.is_alphanumeric());
        
        if self.policy.require_uppercase && !has_upper {
            return Err(PasswordError::MissingUppercase);
        }
        
        if self.policy.require_lowercase && !has_lower {
            return Err(PasswordError::MissingLowercase);
        }
        
        if self.policy.require_numbers && !has_digit {
            return Err(PasswordError::MissingNumber);
        }
        
        if self.policy.require_special && !has_special {
            return Err(PasswordError::MissingSpecial);
        }
        
        // Unique character count
        let unique_chars: HashSet<char> = password.chars().collect();
        if unique_chars.len() < self.policy.min_unique_chars {
            return Err(PasswordError::NotEnoughUniqueChars(self.policy.min_unique_chars));
        }
        
        // Check for username in password
        if self.policy.disallow_username_in_password && !username.is_empty() {
            let password_lower = password.to_lowercase();
            let username_lower = username.to_lowercase();
            if password_lower.contains(&username_lower) || username_lower.contains(&password_lower) {
                return Err(PasswordError::ContainsUsername);
            }
        }
        
        // Check consecutive characters
        if self.policy.max_consecutive_chars > 0 {
            if has_consecutive_chars(password, self.policy.max_consecutive_chars) {
                return Err(PasswordError::TooManyConsecutive(self.policy.max_consecutive_chars));
            }
        }
        
        // Check common passwords
        if self.policy.disallow_common_passwords {
            if COMMON_PASSWORDS.contains(password.to_lowercase().as_str()) {
                return Err(PasswordError::CommonPassword);
            }
        }
        
        // Calculate password strength score
        let strength = calculate_strength(password);
        if strength < 3 {
            return Err(PasswordError::TooWeak);
        }
        
        Ok(())
    }
}

fn has_consecutive_chars(password: &str, max: usize) -> bool {
    let chars: Vec<char> = password.chars().collect();
    
    if chars.len() <= max {
        return false;
    }
    
    for i in 0..chars.len().saturating_sub(max) {
        // Check for sequential characters (abc, 123)
        let mut is_sequential = true;
        for j in 1..=max {
            if i + j >= chars.len() {
                is_sequential = false;
                break;
            }
            // Check if characters are sequential
            if chars[i + j] as u32 != chars[i] as u32 + j as u32 {
                is_sequential = false;
                break;
            }
        }
        if is_sequential {
            return true;
        }
        
        // Check for repeated characters (aaa, 111)
        let mut all_same = true;
        for j in 1..=max {
            if i + j >= chars.len() || chars[i + j] != chars[i] {
                all_same = false;
                break;
            }
        }
        if all_same {
            return true;
        }
    }
    
    false
}

fn calculate_strength(password: &str) -> u32 {
    let mut score = 0;
    
    // Length bonus
    score += match password.len() {
        0..=7 => 0,
        8..=11 => 1,
        12..=15 => 2,
        16..=19 => 3,
        _ => 4,
    };
    
    // Character diversity
    let has_upper = password.chars().any(|c| c.is_uppercase());
    let has_lower = password.chars().any(|c| c.is_lowercase());
    let has_digit = password.chars().any(|c| c.is_numeric());
    let has_special = password.chars().any(|c| !c.is_alphanumeric());
    
    score += [has_upper, has_lower, has_digit, has_special].iter().filter(|&&x| x).count() as u32;
    
    // Entropy estimate
    let unique_chars: HashSet<char> = password.chars().collect();
    if unique_chars.len() > 10 {
        score += 1;
    }
    
    score
}

#[derive(Debug, thiserror::Error)]
pub enum PasswordError {
    #[error("Password must be at least {0} characters")]
    TooShort(usize),
    
    #[error("Password must not exceed {0} characters")]
    TooLong(usize),
    
    #[error("Password must contain an uppercase letter")]
    MissingUppercase,
    
    #[error("Password must contain a lowercase letter")]
    MissingLowercase,
    
    #[error("Password must contain a number")]
    MissingNumber,
    
    #[error("Password must contain a special character")]
    MissingSpecial,
    
    #[error("Password must have at least {0} unique characters")]
    NotEnoughUniqueChars(usize),
    
    #[error("Password must not contain your username")]
    ContainsUsername,
    
    #[error("Password has too many consecutive characters (max {0})")]
    TooManyConsecutive(usize),
    
    #[error("This password is too common")]
    CommonPassword,
    
    #[error("Password is too weak")]
    TooWeak,
}

impl PasswordError {
    pub fn user_message(&self) -> String {
        match self {
            Self::TooShort(min) => format!(
                "Your password needs to be at least {} characters long. Try using a passphrase!",
                min
            ),
            Self::CommonPassword => 
                "This password is too common and easily guessed. Try adding more words or numbers.".to_string(),
            Self::TooWeak => 
                "Your password needs to be stronger. Try making it longer or adding different types of characters.".to_string(),
            _ => self.to_string(),
        }
    }
    
    pub fn suggestions(&self) -> Vec<&'static str> {
        match self {
            Self::TooShort(_) | Self::TooWeak => vec![
                "Use a passphrase: combine 4+ random words",
                "Make it personal but not guessable",
                "Consider using a password manager",
            ],
            Self::CommonPassword => vec![
                "Add numbers or symbols to make it unique",
                "Combine multiple unrelated words",
                "Avoid dictionary words and common substitutions",
            ],
            _ => vec![],
        }
    }
}

pub enum PasswordStrength {
    VeryWeak,   // Score 0-1
    Weak,       // Score 2
    Fair,       // Score 3-4  
    Strong,     // Score 5-6
    VeryStrong, // Score 7+
}

impl PasswordStrength {
    pub fn from_password(password: &str) -> Self {
        let score = calculate_strength(password);
        match score {
            0..=1 => Self::VeryWeak,
            2 => Self::Weak,
            3..=4 => Self::Fair,
            5..=6 => Self::Strong,
            _ => Self::VeryStrong,
        }
    }
    
    pub fn color(&self) -> &'static str {
        match self {
            Self::VeryWeak => "red",
            Self::Weak => "orange",
            Self::Fair => "yellow",
            Self::Strong => "light-green",
            Self::VeryStrong => "green",
        }
    }
    
    pub fn label(&self) -> &'static str {
        match self {
            Self::VeryWeak => "Very Weak",
            Self::Weak => "Weak",
            Self::Fair => "Fair",
            Self::Strong => "Strong",
            Self::VeryStrong => "Very Strong",
        }
    }
}

/// Validate username format
pub fn validate_username(username: &str) -> Result<(), ValidationError> {
    if !USERNAME_REGEX.is_match(username) {
        return Err(ValidationError::new("invalid_username_format"));
    }
    Ok(())
}

/// Validate email format
pub fn validate_email(email: &str) -> Result<(), ValidationError> {
    if !EMAIL_REGEX.is_match(email) {
        return Err(ValidationError::new("invalid_email_format"));
    }
    
    // Additional check for dangerous characters
    if email.contains('<') || email.contains('>') || email.contains('"') || email.contains('\'') {
        return Err(ValidationError::new("email_contains_dangerous_chars"));
    }
    
    Ok(())
}

/// Validate API key name
pub fn validate_api_key_name(name: &str) -> Result<(), ValidationError> {
    if name.is_empty() {
        return Err(ValidationError::new("api_key_name_empty"));
    }
    
    if name.len() > 100 {
        return Err(ValidationError::new("api_key_name_too_long"));
    }
    
    // Check for dangerous characters
    if name.contains('<') || name.contains('>') || name.contains(';') {
        return Err(ValidationError::new("api_key_name_invalid_chars"));
    }
    
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_password_validation() {
        let validator = PasswordValidator::with_default_policy();
        
        // Valid passwords
        assert!(validator.validate("ValidPass123", "user").is_ok());
        assert!(validator.validate("MySecurePassword2024", "user").is_ok());
        
        // Invalid passwords
        assert!(validator.validate("short", "user").is_err());
        assert!(validator.validate("alllowercase123", "user").is_err());
        assert!(validator.validate("ALLUPPERCASE123", "user").is_err());
        assert!(validator.validate("NoNumbersHere", "user").is_err());
    }
    
    #[test]
    fn test_consecutive_chars() {
        assert!(has_consecutive_chars("abcd123", 3));
        assert!(has_consecutive_chars("aaa123", 2));
        assert!(!has_consecutive_chars("AbCd123", 3));
    }
}
