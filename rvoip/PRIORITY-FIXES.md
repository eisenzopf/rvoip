# 🚨 PRIORITY FIXES - Runtime Issues Tracker

## Overview
This document tracks critical runtime issues identified during SIP session testing that need to be resolved for production readiness.

---

## 🔴 **HIGH PRIORITY** - Critical Issues

### **1. Timer E Retransmissions Issue**
- **Status**: 🔴 OPEN
- **Priority**: P0 - Critical
- **Component**: `transaction-core`, `dialog-core`
- **Issue**: INFO and UPDATE requests are being retransmitted multiple times via Timer E
- **Evidence**: 
  ```
  Received command: Timer("E") for transaction Key(...:INFO:client)
  Received command: Timer("E") for transaction Key(...:UPDATE:client)
  ```
- **Root Cause**: Responses aren't being processed quickly enough or aren't reaching client transactions
- **Impact**: Network efficiency degradation, potential response handling failures
- **Tasks**:
  - [ ] Investigate INFO request/response timing in transaction-core
  - [ ] Analyze UPDATE request/response flow for delays
  - [ ] Check if 200 OK responses are properly routed to client transactions
  - [ ] Verify Timer E configuration and response timeouts
  - [ ] Test with faster response processing

### **2. INVITE Transaction Timeout Warnings**
- **Status**: 🔴 OPEN  
- **Priority**: P0 - Critical
- **Component**: `dialog-core`, `transaction-core`
- **Issue**: INVITE transactions timing out during termination despite claiming "normal for 2xx responses"
- **Evidence**:
  ```
  WARN rvoip_dialog_core::manager::unified: INVITE transaction terminated (this is normal for 2xx responses): 
  Transaction error: Failed to send request: SIP transport error: Protocol error: Transaction terminated after timeout
  ```
- **Root Cause**: Likely premature transaction cleanup or improper RFC 3261 timeout handling
- **Impact**: RFC compliance concerns, potential call setup reliability issues
- **Tasks**:
  - [ ] Review RFC 3261 Section 17.1.1.3 for 2xx INVITE transaction behavior
  - [ ] Analyze transaction termination timing in dialog-core
  - [ ] Check if ACK processing affects transaction lifetime
  - [ ] Verify proper transaction state transitions for 2xx responses
  - [ ] Remove misleading "this is normal" warning if it's actually an error

---

## 🟡 **MEDIUM PRIORITY** - Important Issues

### **3. Unknown SDP Event Type Handler Missing**
- **Status**: 🟡 OPEN
- **Priority**: P1 - Important  
- **Component**: `session-core`
- **Issue**: `final_negotiated_sdp` event type is not handled
- **Evidence**:
  ```
  DEBUG rvoip_session_core::manager::core: Unknown SDP event type: final_negotiated_sdp
  ```
- **Root Cause**: Missing event handler for `final_negotiated_sdp` in session manager
- **Impact**: Incomplete SDP processing, potential media negotiation gaps
- **Tasks**:
  - [ ] Add handler for `final_negotiated_sdp` event type in session-core
  - [ ] Determine what processing should occur for this event
  - [ ] Test SDP negotiation completeness after fix
  - [ ] Document all supported SDP event types

### **4. Call State Management UX Improvement**
- **Status**: 🟡 OPEN
- **Priority**: P2 - Moderate
- **Component**: `session-core`
- **Issue**: Resume operation fails with confusing error when call is not on hold
- **Evidence**:
  ```
  WARN simple_peer_to_peer: Resume operation failed (expected in test): Invalid state: Cannot resume call not on hold: Active
  ```
- **Root Cause**: State validation logic could provide more user-friendly messaging
- **Impact**: Poor user experience, unclear error messages
- **Tasks**:
  - [ ] Improve error message for resume-when-not-on-hold scenario
  - [ ] Add state transition validation with helpful hints
  - [ ] Consider allowing no-op resume for active calls
  - [ ] Review all state management error messages for clarity

---

## 🟢 **LOW PRIORITY** - Code Quality Issues

### **5. Compilation Warnings Cleanup**
- **Status**: 🟢 OPEN
- **Priority**: P3 - Low
- **Component**: Multiple crates
- **Issue**: Multiple unused import warnings across codebase
- **Evidence**:
  ```
  warning: unused imports: `EventError`, `StaticEvent`, `Error`, `Result`, `DialogError`
  ```
- **Root Cause**: Leftover imports from refactoring and development
- **Impact**: Code cleanliness, compilation noise
- **Tasks**:
  - [ ] Remove unused `EventError` imports from event modules
  - [ ] Clean up unused `StaticEvent` imports
  - [ ] Remove unused `Error` and `Result` imports from error modules
  - [ ] Remove unused `DialogError` import
  - [ ] Run `cargo clippy` to catch any other unused imports

---

## 📊 **Progress Tracking**

### **Summary**
- **Total Issues**: 5
- **Critical (P0)**: 2 🔴
- **Important (P1)**: 1 🟡  
- **Moderate (P2)**: 1 🟡
- **Low (P3)**: 1 🟢

### **Next Actions**
1. **Immediate**: Fix Timer E retransmissions (blocking network efficiency)
2. **Next**: Resolve transaction timeout warnings (RFC compliance)
3. **Then**: Add missing SDP event handler (feature completeness)
4. **Finally**: Code cleanup and UX improvements

### **Dependencies**
- Timer E fix may require transaction-core and dialog-core coordination
- Transaction timeout fix needs careful RFC 3261 compliance review
- SDP event handler is isolated and can be fixed independently

---

## 📝 **Notes**
- All issues identified from runtime log analysis of `simple_peer_to_peer` example
- System is functionally working but has efficiency and compliance issues
- Focus on P0 issues first for production readiness
