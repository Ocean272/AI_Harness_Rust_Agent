# Rust Agent (Local AI & Infrastructure Service)

A high-performance, local-first Rust autonomous agent service built on **Axum**, **Qdrant**, **Ollama**, and **Docker**. The system combines vector memory retrieval (RAG), background listeners, custom micro-GPT transformer mechanisms, a Human-in-the-Loop (HITL) approval workflow, and an isolated Docker execution sandbox.

---

## 🌟 Key Features

* **Modular Crate Architecture**: Clean separation across `handlers/`, `listeners/`, `monitor/`, `services/`, and `tools/` modules.
* **Vector Memory & RAG Integration**: Semantic search and memory ingestion backed by Qdrant vector storage (`qdrant.rs`, `rag.rs`).
* **In-House Transformer & GPT Engine**: Lightweight local inference and causal self-attention modules (`attention.rs`, `micro_gpt.rs`).
* **Human-In-The-Loop (HITL) Gatekeeper**: Intercepts high-risk or infrastructure-modifying commands into a persistent SQLite task queue (`db.rs`) awaiting manual approval.
* **Docker Sandboxed Execution**: Runs system-level commands inside ephemeral, network-isolated (`--network none`) Alpine containers for safe host execution (`executor.rs`, `sanitizer.rs`).
* **Multi-Channel Listeners**: Integrated automated background listeners for Cron, IMAP email triggers, and file system watchers (`listeners/`).
* **Web UI Dashboards**: Embedded HTML frontend (`static/index.html`, `search.html`, `submit.html`) for task submission, monitoring, and vector memory retrieval.

---

## 🏗️ System Architecture
                                 ┌──────────────────────────────┐
                                 │       Axum Web Server        │
                                 │  (static/ & REST API Routes) │
                                 └──────────────┬───────────────┘
                                                │
         ┌──────────────────────────────────────┼───────────────────────────────────┐
         ▼                                      ▼                                   ▼
┌─────────────────────────┐       ┌──────────────────────────┐         ┌─────────────────────────┐
│     Vector RAG & AI     │       │   HITL & Storage Engine  │         │   Background Watchers   │
│ (Qdrant, attention.rs,  │       │(db.rs SQLite Persistence,│         │  (File Watcher, IMAP,   │
│  micro_gpt.rs, Ollama)  │       │   security.rs Filter)    │         │       Cron, Cloud)      │
└─────────────────────────┘       └─────────────┬────────────┘         └─────────────────────────┘
                                                │
                                    [ Is High-Risk Command? ]
                                            /       \
                                           /         \
                                         Yes          No
                                        /               \
                                       /                 \
                            ┌─────────┴───────┐     ┌─────┴─────────────────────┐
                            │ Pending Queue   │     │ Docker Execution Sandbox  │
                            │ (Awaits HITL)   │     │ (alpine, --network none)  │
                            └─────────────────┘     └───────────────────────────┘

---

## 📁 Repository Structure

```text
rust-agent/
├── .cargo/
├── .github/
├── data/
├── inbox/
├── models/
├── src/
│   ├── handlers/            # Axum request handlers
│   │   ├── command.rs       # Task submission and execution handlers
│   │   ├── mod.rs           # Module exports
│   │   └── rag.rs           # Vector memory query handlers
│   ├── inbox/               # Ingested email trigger payload directory
│   ├── listeners/           # Automated event listeners
│   │   ├── file_watcher.rs  # Directory watcher listener
│   │   ├── imap.rs          # Email inbox listener
│   │   └── mod.rs
│   ├── monitor/             # System and node monitoring
│   │   ├── cloud.rs         # Remote/Cloud health monitor
│   │   ├── embedded.rs      # Local embedded monitor
│   │   └── mod.rs
│   ├── services/            # Core agent logic and sandbox services
│   │   ├── agent.rs         # Main orchestration agent logic
│   │   ├── bootstrap.rs     # Background worker initialization
│   │   ├── cron.rs          # Scheduled cron service
│   │   ├── executor.rs      # Command execution engine
│   │   ├── micro_gpt.rs     # Transformer model handler
│   │   ├── mod.rs
│   │   ├── model.rs         # Model states & domain entities
│   │   └── sanitizer.rs     # Input cleaning & command parsing
│   ├── tools/               # Agent tool implementations
│   │   ├── email.rs         # Email sending/parsing tools
│   │   ├── mod.rs
│   │   ├── qdrant_tools.rs  # Vector database utilities
│   │   ├── security.rs      # Command blocklist & security checks
│   │   └── terminal.rs      # Shell terminal interaction tools
│   ├── api.rs               # Axum AppState and router configuration
│   ├── attention.rs         # Causal self-attention & tensor logic
│   ├── db.rs                # SQLite database persistence layer
│   ├── lib.rs               # Crate root module declarations
│   ├── main.rs              # Application entrypoint
│   └── qdrant.rs            # Qdrant client vector database service
├── static/                  # Static Web UI frontend
│   ├── index.html           # Main monitoring dashboard
│   ├── search.html          # Vector memory search view
│   └── submit.html          # Task submission interface
├── .env                     # Local environment configuration
├── .gitignore
├── Cargo.lock
├── Cargo.toml               # Crate dependencies
├── load_test.sh             # API load testing script
└── readme.md                # Project documentation



🛡️ Sandbox Execution & Safety

To keep host hypervisors safe, all executed terminal commands pass through two security layers:

1. Security Command Filter (tools/security.rs): High-privilege or dangerous commands are flagged and held in SQLite (db.rs) for human review.

2. Container Isolation (services/executor.rs): Approved shell executions run within an isolated Docker container:
* Network Isolation: Spun up with --network none to prevent unintended network access.
* Ephemeral Lifecycle: Launched with --rm to automatically destroy container state upon exit.
* Output Interception: stdout and stderr are clean-wrapped and logged back into the task trace.



🚀 Getting Started

Prerequisites
- Rust (1.75+ recommended)
- Docker Engine (running local daemon)
- Qdrant Vector DB (running on port 6333 / 6334)
- Ollama (for local embedding model inference)

Building & Execution
- Clone the repository:

Bash

git clone [https://github.com/your-username/rust-agent.git](https://github.com/your-username/rust-agent.git)
cd rust-agent


Configure Environment:
Ensure .env contains your database and endpoint settings:Code snippetDATABASE_URL=sqlite://tasks.db
QDRANT_URL=http://localhost:6334
OLLAMA_URL=http://localhost:11434
RUST_LOG=info

Check & Compile:
Bash

# Verify crate visibility across lib and bin targets
cargo check --all-targets

# Build and launch agent service
cargo run



📡 API Endpoints

Method              Route                           Description
GE                  /                               Serves main Dashboard UI (index.html)
GET                 /search                         Serves Vector Search UI (search.html)
GET                 /submit                         Serves Task Submission UI (submit.html)
POST                /api/memory/search              Queries Qdrant vector memory snippets (RAG)
POST                /api/v1/agent/task              Submits a command or agent task
GET                 /api/v1/tasks/pending           Retrieves held HITL approval tasks
POST                /api/v1/tasks/:task_id/approve  Approves and executes a pending task
POST                /api/ingest                     Ingests new text memory into Qdrant



🧪 Testing
Run automated load tests and unit test suites:
Bash

# Run Rust unit tests
cargo test

# Run HTTP route load tests
bash 
load_test.sh


📄 License
Distributed under the MIT License.