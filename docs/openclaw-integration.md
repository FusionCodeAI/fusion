# OpenClaw + Fusion Integration Guide

This guide describes how to connect the **OpenClaw AI Gateway** (for WhatsApp/Telegram messaging and dashboard control) with the **Fusion CLI** (for safe, high-speed terminal execution) inside an **Ubuntu** environment (on Android Termux or iOS UTM).

---

## Architecture Overview

In this setup, OpenClaw acts as your communication hub, while Fusion serves as the secure local command executor:

```
[📱 User via Chat] <--> [OpenClaw (Ubuntu Node.js)] <--> [Fusion CLI (Rust Binary)] <--> [Filesystem]
```

---

## 1. Setup the Ubuntu Environment

First, install and enter a standard Ubuntu environment on your device:

### On Android (Termux PRoot)
```bash
pkg install proot-distro
proot-distro install ubuntu
proot-distro login ubuntu
```

### On iOS (UTM Virtual Machine)
Create a new VM using the official **Ubuntu Server** ISO, set up your user account, and log in.

---

## 2. Install OpenClaw & Dependencies

Inside your Ubuntu shell, install Node.js and the global OpenClaw gateway:

```bash
# Update Ubuntu package list
apt update && apt upgrade -y
apt install curl git ripgrep ca-certificates build-essential -y

# Install Node.js 22
curl -fsSL https://deb.nodesource.com/setup_22.x | bash -
apt install -y nodejs

# Install OpenClaw
npm install -g openclaw@latest
```

---

## 3. Install Fusion

Install the latest self-contained version of Fusion inside the same Ubuntu shell:

```bash
curl -fsSL https://fusioncode.app/install | bash
```

Verify the installation:
```bash
fusion --version
```

---

## 4. Run the OpenClaw Onboarding

Link your API keys and pair your chat channels (WhatsApp/Telegram):

```bash
openclaw onboard
```
*Note: Select **Loopback (127.0.0.1)** for the Gateway Bind to avoid permission crashes.*

---

## 5. Connecting OpenClaw to Fusion

OpenClaw allows the agent to execute shell commands. To ensure the OpenClaw agent uses Fusion for code modifications (guaranteeing safety and exact matching), instruct OpenClaw to delegate file edits and search-replace tasks to the `fusion` binary.

### Option A: System Prompt Delegation
During the onboarding or in your OpenClaw custom agent system instructions, append the following directive:

```text
You have access to the 'fusion' CLI tool on the system.
For any multi-file code exploration, precise search-replace edits, and compilation check tasks:
Run the command: fusion -p "<detailed description of the task>"
This will run a sub-agent that executes the edits safely.
```

### Option B: Custom OpenClaw Bash Tool
You can write a custom OpenClaw shell tool (`fusion-tool.sh`) that wraps the CLI:

```bash
#!/bin/bash
# Save as ~/.openclaw/tools/fusion-tool.sh
# chmod +x ~/.openclaw/tools/fusion-tool.sh
fusion -p "$1"
```

---

## 6. Launch the Gateway

Start your gateway server:

```bash
openclaw gateway --verbose
```

You can now text your bot on WhatsApp/Telegram! When you ask it to write code or modify files, it will run the high-speed Rust-based `fusion` binary on the host filesystem and reply with the results.
