#!/usr/bin/env bash
# ═══════════════════════════════════════════════════════════════════
# tredo — One-Line Installation Script
# ═══════════════════════════════════════════════════════════════════
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/craaraju-ctrl/Tredo/main/install.sh | bash
#   curl -fsSL https://raw.githubusercontent.com/craaraju-ctrl/Tredo/main/install.sh | bash -s -- --help
#
# Options:
#   --help              Show this help
#   --branch <name>     Git branch to clone (default: main)
#   --dir <path>        Target directory (default: ./tredo)
#   --no-wizard         Skip interactive setup wizard (just clone + build)
#   --minimal           Skip Kronos/Python/Ollama setup, Rust + build only
#   --release           Build with --release (slower, optimized binary)
# ═══════════════════════════════════════════════════════════════════

set -euo pipefail

# ── Constants ──────────────────────────────────────────────────────────────
REPO_URL="https://github.com/craaraju-ctrl/Tredo.git"
REPO_BRANCH="main"
INSTALL_DIR="./tredo"
RUN_WIZARD=true
MINIMAL=false
BUILD_MODE=""
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

# ── Output Helpers ─────────────────────────────────────────────────────────
step()   { echo -e "\n${BLUE}╔══════════════════════════════════════════════════════════╗${NC}"; echo -e "${BLUE}║${NC}  ${BOLD}STEP $((++STEP_COUNT)):${NC} $1"; echo -e "${BLUE}╚══════════════════════════════════════════════════════════╝${NC}"; }
info()   { echo -e "  ${CYAN}→${NC} $1"; }
ok()     { echo -e "  ${GREEN}✓${NC} $1"; }
warn()   { echo -e "  ${YELLOW}⚠${NC} $1"; }
fail()   { echo -e "  ${RED}✗${NC} $1"; }
header() { echo -e "\n${BOLD}$1${NC}"; }
STEP_COUNT=0

# ── Parse Arguments ────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
  case "$1" in
    --help|-h)
      sed -n '3,/^set -e/p' "${BASH_SOURCE[0]}" | grep -E '^#|^$' | sed 's/^# //; s/^#$//' | head -20
      exit 0 ;;
    --branch) REPO_BRANCH="$2"; shift 2 ;;
    --dir) INSTALL_DIR="$2"; shift 2 ;;
    --no-wizard) RUN_WIZARD=false; shift ;;
    --minimal) MINIMAL=true; shift ;;
    --release) BUILD_MODE="--release"; shift ;;
    *) echo -e "${RED}Unknown option: $1${NC}"; exit 1 ;;
  esac
done

# ── Preflight: OS & Arch ───────────────────────────────────────────────────
step "System Check"
OS="$(uname -s)"
ARCH="$(uname -m)"
info "Detected: $OS / $ARCH"

case "$OS" in
  Linux|Darwin) ok "Supported OS: $OS" ;;
  *) fail "Unsupported OS: $OS (Linux or macOS required)"; exit 1 ;;
esac

case "$ARCH" in
  x86_64|aarch64|arm64) ok "Supported architecture: $ARCH" ;;
  *) warn "Untested architecture: $ARCH (may still work)" ;;
esac

# Check for required tools
MISSING=""
for cmd in git curl; do
  if ! command -v "$cmd" &>/dev/null; then
    MISSING="$MISSING $cmd"
  fi
done
if [ -n "$MISSING" ]; then
  fail "Missing required tools:$MISSING"
  info "Install them first:"
  if [ "$OS" = "Linux" ]; then
    echo "    sudo apt update && sudo apt install -y$MISSING   (Debian/Ubuntu)"
    echo "    sudo yum install -y$MISSING                     (RHEL/Fedora)"
  else
    echo "    brew install$MISSING                            (macOS)"
  fi
  exit 1
fi
ok "git and curl available"

# ── Step 2: Clone Repository ───────────────────────────────────────────────
step "Clone tredo Repository"

if [ -d "$INSTALL_DIR/.git" ]; then
  info "Directory '$INSTALL_DIR' already exists. Updating..."
  cd "$INSTALL_DIR"
  git fetch origin "$REPO_BRANCH"
  git reset --hard "origin/$REPO_BRANCH"
  cd ..
  ok "Updated to latest $REPO_BRANCH"
else
  if [ -d "$INSTALL_DIR" ]; then
    warn "Directory '$INSTALL_DIR' exists but is not a git repo."
    read -p "  Remove and re-clone? [y/N] " ans
    if [[ "$ans" =~ ^[Yy] ]]; then
      rm -rf "$INSTALL_DIR"
    else
      fail "Please choose a different directory with --dir or remove it manually."
      exit 1
    fi
  fi
  info "Cloning $REPO_URL (branch: $REPO_BRANCH)..."
  git clone --depth=1 --branch "$REPO_BRANCH" "$REPO_URL" "$INSTALL_DIR"
  ok "Repository cloned to $INSTALL_DIR"
fi

cd "$INSTALL_DIR"
INSTALL_DIR_ABS="$(pwd)"

# ── Step 3: Install Rust (if needed) ────────────────────────────────────────
step "Rust Toolchain"

install_rust() {
  info "Installing Rust via rustup..."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --component rustfmt --component clippy
  source "${HOME}/.cargo/env"
  ok "Rust installed"
}

if command -v rustc &>/dev/null; then
  RUST_VERSION="$(rustc --version | sed -n 's/^rustc \([0-9]\+\.[0-9]\+\).*/\1/p')"
  info "Found Rust $RUST_VERSION (minimum required: 1.75)"
  # Parse major.minor as integers for portable version comparison
  RUST_MAJOR="${RUST_VERSION%%.*}"
  RUST_MINOR="${RUST_VERSION#*.}"
  if [ -z "$RUST_VERSION" ]; then
    warn "Could not parse Rust version. Upgrading..."
    rustup update stable
  elif [ "$RUST_MAJOR" -gt 1 ] || { [ "$RUST_MAJOR" -eq 1 ] && [ "$RUST_MINOR" -ge 75 ]; }; then
    ok "Rust $RUST_VERSION meets minimum requirement (1.75)"
  else
    warn "Rust $RUST_VERSION is below 1.75. Upgrading..."
    rustup update stable
    ok "Rust upgraded to $(rustc --version)"
  fi
else
  warn "Rust not found. Installing..."
  install_rust
fi

# Ensure clippy + rustfmt are available
rustup component add clippy rustfmt 2>/dev/null || true
ok "Toolchain ready: $(rustc --version)"

# ── Step 4: Copy Environment Template ──────────────────────────────────────
step "Environment Configuration"

CONFIG_DIR="config"
CONFIG_FILE="$CONFIG_DIR/tredo.env"
mkdir -p "$CONFIG_DIR"

if [ -f "$CONFIG_FILE" ]; then
  info "Existing config found at $CONFIG_FILE"
  read -p "  Overwrite? [y/N] " ans
  if [[ "$ans" =~ ^[Yy] ]]; then
    cp "$CONFIG_DIR/tredo.env.example" "$CONFIG_FILE"
    ok "Config template copied (edit $CONFIG_FILE with your API keys)"
  else
    ok "Keeping existing configuration"
  fi
else
  cp "$CONFIG_DIR/tredo.env.example" "$CONFIG_FILE"
  ok "Config template copied to $CONFIG_FILE"
  info "Edit it with your API keys before running:"
  info "    nano $CONFIG_FILE"
fi

# ── Step 5: Build Project ──────────────────────────────────────────────────
step "Build tredo (cargo build)"

BUILD_TARGETS="-p tredo-core -p tredo-autonomous -p tredo-orchestrator -p tredo-tui"
info "Building with: cargo build $BUILD_MODE $BUILD_TARGETS"
info "This may take several minutes (compiling ~80+ Rust crates)..."

START_TS=$SECONDS
set +e
if [ -n "$BUILD_MODE" ]; then
  cargo build $BUILD_MODE $BUILD_TARGETS 2>&1 | tail -5
else
  cargo build $BUILD_TARGETS 2>&1 | tail -5
fi
BUILD_STATUS="${PIPESTATUS[0]}"
set -e
ELAPSED=$((SECONDS - START_TS))

if [ "$BUILD_STATUS" = "0" ]; then
  ok "Build completed successfully in ${ELAPSED}s"
  # Show binary sizes
  for bin in tredo-orchestrator tredo-tui; do
    BIN_PATH="target/debug/$bin"
    [ -n "$BUILD_MODE" ] && BIN_PATH="target/release/$bin"
    if [ -f "$BIN_PATH" ]; then
      SIZE="$(du -h "$BIN_PATH" | cut -f1)"
      info "  $bin  ($SIZE)"
    fi
  done
else
  fail "Build failed. Check output above."
  exit 1
fi

# ── Step 6: Setup Wizard (Interactive) ─────────────────────────────────────
if [ "$RUN_WIZARD" = true ] && [ "$MINIMAL" = false ]; then
  step "Setup Wizard"

  echo -e "${CYAN}  The wizard will help configure:${NC}"
  echo -e "${CYAN}    • LLM provider (Ollama recommended for local use)${NC}"
  echo -e "${CYAN}    • Exchange API keys (dummy values for paper trading)${NC}"
  echo -e "${CYAN}    • News API keys (optional, free tiers available)${NC}"
  echo -e "${CYAN}    • WebSocket & server settings${NC}"
  echo -e "${CYAN}    • Trading symbols watchlist${NC}"
  echo
  read -p "  Run the setup wizard now? [Y/n] " ans
  if [[ ! "${ans:-Y}" =~ ^[Nn] ]]; then
    bash "$INSTALL_DIR_ABS/tredo" setup
  else
    info "Skipping wizard. Edit $CONFIG_FILE manually and then:"
    info "    source $CONFIG_FILE"
    info "    $INSTALL_DIR_ABS/tredo tui"
  fi
elif [ "$MINIMAL" = true ]; then
  info "Minimal install — skipping optional dependencies wizard."
fi

# ── Step 7: Optional Dependencies ─────────────────────────────────────────
if [ "$MINIMAL" = false ]; then
  step "Optional Dependencies"

  # ── Python + Kronos ────────────────────────────────────────────────────
  if command -v python3 &>/dev/null; then
    PY_VERSION="$(python3 --version 2>&1 | sed -n 's/^Python \([0-9]\+\.[0-9]\+\).*/\1/p')"
    info "Python $PY_VERSION found"
    PY_MAJOR="${PY_VERSION%%.*}"
    PY_MINOR="${PY_VERSION#*.}"
    if [ -z "$PY_VERSION" ]; then
      warn "Could not parse Python version"
    elif [ "$PY_MAJOR" -gt 3 ] || { [ "$PY_MAJOR" -eq 3 ] && [ "$PY_MINOR" -ge 10 ]; }; then
      read -p "  Install Kronos forecast service dependencies? [Y/n] " ans
      if [[ ! "${ans:-Y}" =~ ^[Nn] ]]; then
        info "Installing Kronos Python dependencies..."
        cd kronos_service
        if command -v uv &>/dev/null; then
          uv pip install -r requirements.txt 2>&1 | tail -3
        else
          python3 -m pip install -r requirements.txt 2>&1 | tail -3
        fi
        cd "$INSTALL_DIR_ABS"
        ok "Kronos dependencies installed"
        info "  Start Kronos: cd kronos_service && uvicorn main:app --port 8000"
      fi
    else
      warn "Python $PY_VERSION is below 3.10. Kronos requires 3.10+."
    fi
  else
    warn "Python 3 not found. Kronos forecast service requires Python 3.10+."
    info "  Install via: brew install python (macOS) or apt install python3 (Linux)"
  fi

  # ── Ollama ──────────────────────────────────────────────────────────────
  if command -v ollama &>/dev/null; then
    ok "Ollama found: $(ollama --version 2>/dev/null || echo 'installed')"
    read -p "  Pull recommended LLM model (ministral:3b, ~2GB)? [y/N] " ans
    if [[ "$ans" =~ ^[Yy] ]]; then
      info "Pulling ministral:3b (this may take a while)..."
      ollama pull ministral:3b
      ok "Model pulled"
    fi
  else
    warn "Ollama not found (required for local LLM inference)."
    info "  Install: curl -fsSL https://ollama.com/install.sh | sh"
    info "  Then: ollama serve & && ollama pull ministral:3b"
  fi
fi

# ── Step 8: Create Symlink (Optional) ─────────────────────────────────────
step "Finalize Installation"

# Create a convenient symlink
SYMLINK_DIR="${HOME}/.local/bin"
if [ -d "$SYMLINK_DIR" ] && [[ ":$PATH:" == *":$SYMLINK_DIR:"* ]]; then
  ln -sf "$INSTALL_DIR_ABS/tredo" "$SYMLINK_DIR/tredo"
  ok "Symlink created: $SYMLINK_DIR/tredo → tredo"
else
  info "Add tredo to your PATH:"
  info "    export PATH=\"\$PATH:$INSTALL_DIR_ABS\""
  info "    alias tredo='$INSTALL_DIR_ABS/tredo'"
fi

# ── Print Summary ─────────────────────────────────────────────────────────
echo
echo -e "${GREEN}╔══════════════════════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║${NC}  ${BOLD}tredo Installation Complete!${NC}               ${GREEN}║${NC}"
echo -e "${GREEN}╚══════════════════════════════════════════════════════════╝${NC}"
echo
echo -e "  ${BOLD}Installed at:${NC}  $INSTALL_DIR_ABS"
echo -e "  ${BOLD}Config file:${NC}   $INSTALL_DIR_ABS/$CONFIG_FILE"
echo
echo -e "  ${BOLD}Quick Start:${NC}"
echo "    cd $INSTALL_DIR_ABS"
echo "    source $CONFIG_FILE"
echo "    ./tredo tui"
echo
echo -e "  ${BOLD}Available Commands:${NC}"
echo "    ./tredo tui                  # Launch Terminal UI (primary interface)"
echo "    ./tredo orchestrator          # Start backend only"
echo "    ./tredo setup                 # Re-run setup wizard"
echo "    ./tredo build                 # Build all binaries"
echo "    ./tredo validate              # Real-time paper crypto validation"
echo "    ./tredo validate --extended   # Extended validation run"
echo "    ./tredo validate --long --induce-regret  # Self-evolution demo"
echo
echo -e "  ${BOLD}Background Services:${NC}"
echo "    # Start Ollama (LLM)"
echo "    ollama serve &"
echo
echo "    # Start Kronos (forecast)"
echo "    cd $INSTALL_DIR_ABS/kronos_service"
echo "    uvicorn main:app --port 8000"
echo
echo -e "  ${BOLD}Documentation:${NC}"
echo "    $INSTALL_DIR_ABS/Build.md"
echo "    $INSTALL_DIR_ABS/README.md"
echo
echo -e "  ${YELLOW}⚠  Paper trading only. Not financial advice.${NC}"
echo
