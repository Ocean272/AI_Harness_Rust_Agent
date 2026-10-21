# Rust-Agent :: Hybrid Autonomous Infrastructure Control Plane

A production-grade, event-driven autonomous AI agent harness built in Rust. Designed to run on Proxmox VE LXC container nodes, `rust-agent` monitors internal infrastructure event streams (IMAP email, filesystem events, scheduled cron tasks), enforces security isolation, manages vector memory via Qdrant, and executes system tasks with an intelligent local-to-cloud LLM failover strategy.

---

## Architecture Overview

```text
                                  +---------------------------------------+
                                  |         Event Ingestion Layer         |
                                  |  - IMAP Listener (docker-mailserver)  |
                                  |  - File System Watcher                |
                                  |  - Cron Scheduler                     |
                                  +-------------------+-------------------+
                                                      |
                                                      v
+---------------------------------+       +-----------+-----------+
|      Gemini Cloud Fallback      |<------|  Saturation Monitor   |
| (Secondary model / High Context)| >88%  | (>88% Threshold Check)|
+---------------------------------+       +-----------+-----------+
                                                      | <=88%
                                                      v
                                          +-----------+------------+
                                          |Local Inference Engine  |
                                          | (Ollama / Local LLM)   |
                                          +-----------+------------+
                                                      |
                                                      v
                                        +-------------+-------------+
                                        |  Security Gateway / Rules |
                                        |    (Sanitizer & HITL)     |
                                        +-------------+-------------+
                                                      |
                                                      v
                                        +-------------+-------------+
                                        |     Execution Engine      |
                                        | (Terminal Tools / Actions)|
                                        +-------------+-------------+
                                                      |
                                                      v
                                          +-----------+-----------+
                                          |  Vector RAG Ingestion |
                                          |   (Qdrant Vector DB)  |
                                          +-----------------------+

```

---

## Core Capabilities

* **Hybrid Local/Cloud Intelligence:** Prioritizes privacy and speed using local LLMs (via Ollama) for routine tasks. Automatically escalates payloads that breach resource/context limits (>88% saturation) to Gemini cloud models.

* **Event-Driven Listeners:** Continuous background monitoring for new tasks via IMAP mailbox polling, local directory file modifications, and scheduled cron jobs.

* **Security Gateway & HITL Control:** Inspects all proposed commands for dangerous operations (e.g., destructive system flags, unauthorized file modifications). High-risk operations are held in a Human-in-the-Loop (HITL) approval queue before execution.

* **Episodic Vector Memory (RAG):** Automatically embeds task prompts, execution outputs, and status logs into Qdrant for semantic search and context retrieval across runs.

* **Control Plane Web Interface:** Built-in web dashboard served via embedded static assets (`index.html`) providing a real-time audit log, active HITL queue management, and task status metrics.

---

## Repository Structure

```text
rust-agent/
├── src/
│   ├── listeners/          # Async event watchers
│   │   ├── file_watcher.rs # Filesystem event monitoring
│   │   ├── imap.rs         # IMAP mail listener & payload dispatcher
│   │   └── mod.rs
│   ├── monitor/            # Resource & context tracking
│   │   ├── cloud.rs        # Gemini API fallback client
│   │   ├── embedded.rs     # Local model integration
│   │   └── mod.rs          # Saturation calculation logic
│   ├── services/           # Orchestration & execution core
│   │   ├── cron.rs         # Scheduled task triggers
│   │   ├── executor.rs     # Async task dispatch & command execution
│   │   ├── sanitizer.rs    # Shell command security validation
│   │   └── mod.rs
│   ├── tools/              # Action integration modules
│   │   ├── email.rs        # Email parser & payload builder
│   │   ├── qdrant_tools.rs # Vector search & point manipulation
│   │   ├── security.rs     # Command risk scoring logic
│   │   ├── terminal.rs     # Shell tool execution interface
│   │   └── mod.rs
│   ├── api.rs              # REST API endpoints for Web UI & triggers
│   ├── db.rs               # Local state & task store management
│   ├── lib.rs              # Core module bindings
│   ├── main.rs             # Application entry point & service wiring
│   └── qdrant.rs           # Qdrant client connection & collection setup
├── static/
│   └── index.html          # Hybrid Control Plane dashboard UI
├── .env                    # Environment configuration
├── Cargo.toml              # Rust crate dependencies
└── Cargo.lock

```

---

## Key Modules Breakdown

| Module / Directory | Responsibility |
| --- | --- |
| **`listeners/`** | Manages asynchronous ingestion sources. `imap.rs` logs into the mail      server, checks for unseen emails, and dispatches new task payloads to the core API. |
| **`monitor/`** | Evaluates payload size and context saturation. Routes normal requests to local models (`embedded.rs`) and over-capacity requests to cloud LLMs (`cloud.rs`). |
| **`services/`** | Handles pipeline mechanics. `sanitizer.rs` screens commands against security policies, while `executor.rs` manages shell invocation and HITL pauses. |
| **`tools/`** | Defines functions exposed to the LLM agent, including command execution (`terminal.rs`), email processing (`email.rs`), and vector memory operations (`qdrant_tools.rs`). |
| **`api.rs` & `static/**` | Houses the HTTP server and UI endpoints used to render the task audit log, present execution telemetry, and allow manual HITL command approvals. |

---

## Environment Configuration (`.env`)

Create a `.env` file in the root directory prior to running:

```env
# Server Setup
PORT=8080
RUST_LOG=info

# Local Mail Server (docker-mailserver)
IMAP_SERVER=192.168.0.x
IMAP_PORT=143
IMAP_USER=agent@sandbox.local
IMAP_PASS=your_secure_password

# Vector Database (Qdrant)
QDRANT_URL=http://192.168.0.x:6333
QDRANT_COLLECTION=emails

# Cloud Fallback (Gemini)
GEMINI_API_KEY=your_gemini_api_key

```

---

## Getting Started

### Prerequisites

* **Rust:** Nightly or latest Stable toolchain (`cargo`, `rustc`)
* **Qdrant:** Active instance reachable via network or container
* **Ollama:** Running locally with target models loaded
* **Mail Server:** Active IMAP service accessible on network

### Build and Run

1. Clone the repository to your host or LXC container:
```bash
git clone https://github.com/your-username/rust-agent.git
cd rust-agent

```


2. Compile and run in development mode:
```bash
cargo run

```


3. Access the Control Plane Dashboard:
Navigate to `http://<NODE_IP>:8080` in your web browser.