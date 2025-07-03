-- Initial Call Center Database Schema
-- Replaces the previous rusqlite schema with sqlx migration

-- Agents table
CREATE TABLE IF NOT EXISTS agents (
    id INTEGER PRIMARY KEY,
    agent_id TEXT NOT NULL UNIQUE,
    username TEXT NOT NULL,
    contact_uri TEXT,
    last_heartbeat DATETIME,
    status TEXT NOT NULL CHECK (status IN ('AVAILABLE', 'BUSY', 'POSTCALLWRAPUP', 'OFFLINE', 'RESERVED')),
    current_calls INTEGER NOT NULL DEFAULT 0 CHECK (current_calls >= 0),
    max_calls INTEGER NOT NULL DEFAULT 1 CHECK (max_calls > 0),
    available_since DATETIME,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Call queue table
CREATE TABLE IF NOT EXISTS call_queue (
    id INTEGER PRIMARY KEY,
    call_id TEXT NOT NULL UNIQUE,
    session_id TEXT NOT NULL UNIQUE,
    queue_id TEXT NOT NULL,
    customer_info TEXT,
    priority INTEGER NOT NULL DEFAULT 1 CHECK (priority > 0),
    enqueued_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    last_attempt DATETIME,
    expires_at DATETIME NOT NULL
);

-- Active calls table
CREATE TABLE IF NOT EXISTS active_calls (
    id INTEGER PRIMARY KEY,
    call_id TEXT NOT NULL UNIQUE,
    agent_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    customer_dialog_id TEXT,
    agent_dialog_id TEXT,
    assigned_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    answered_at DATETIME,
    FOREIGN KEY (agent_id) REFERENCES agents(agent_id)
);

-- Queues table
CREATE TABLE IF NOT EXISTS queues (
    id INTEGER PRIMARY KEY,
    queue_id TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    description TEXT,
    max_wait_time INTEGER CHECK (max_wait_time > 0),
    priority_routing BOOLEAN DEFAULT FALSE,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- Call records table
CREATE TABLE IF NOT EXISTS call_records (
    id INTEGER PRIMARY KEY,
    call_id TEXT NOT NULL UNIQUE,
    customer_number TEXT,
    agent_id TEXT,
    queue_name TEXT,
    start_time DATETIME,
    end_time DATETIME,
    duration_seconds INTEGER CHECK (duration_seconds >= 0),
    disposition TEXT CHECK (disposition IN ('answered', 'abandoned', 'timeout', 'error')),
    notes TEXT,
    FOREIGN KEY (agent_id) REFERENCES agents(agent_id)
);

-- Performance indexes
CREATE INDEX IF NOT EXISTS idx_agents_status ON agents(status);
CREATE INDEX IF NOT EXISTS idx_agents_status_calls ON agents(status, current_calls);
CREATE INDEX IF NOT EXISTS idx_agents_username ON agents(username);

CREATE INDEX IF NOT EXISTS idx_call_queue_priority ON call_queue(queue_id, priority DESC, enqueued_at);
CREATE INDEX IF NOT EXISTS idx_call_queue_expires ON call_queue(expires_at);

CREATE INDEX IF NOT EXISTS idx_active_calls_agent ON active_calls(agent_id);
CREATE INDEX IF NOT EXISTS idx_active_calls_session ON active_calls(session_id);

CREATE INDEX IF NOT EXISTS idx_call_records_call_id ON call_records(call_id);
CREATE INDEX IF NOT EXISTS idx_call_records_agent_id ON call_records(agent_id);
CREATE INDEX IF NOT EXISTS idx_call_records_start_time ON call_records(start_time);

-- Insert default queues
INSERT OR IGNORE INTO queues (queue_id, name, description, max_wait_time, priority_routing) VALUES
('default', 'Default Queue', 'Default call queue for general inquiries', 300, FALSE),
('support', 'Technical Support', 'Technical support queue for customer issues', 600, TRUE),
('sales', 'Sales Queue', 'Sales and pre-sales inquiries', 180, TRUE),
('billing', 'Billing Support', 'Billing and account related queries', 300, FALSE),
('escalation', 'Escalation Queue', 'Escalated calls requiring supervisor attention', 900, TRUE); 