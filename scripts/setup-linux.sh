#!/usr/bin/env bash
# ==============================================================================
# PhoneCam Linux Environment & Virtual Camera Automated Setup Script
# Supported: Ubuntu / Debian / Fedora / Arch Linux / Manjaro
# ==============================================================================
set -euo pipefail

RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

echo -e "${BLUE}=== PhoneCam Linux One-Click Setup ===${NC}"
echo "Detecting Linux distribution..."

if [ -f /etc/os-release ]; then
    . /etc/os-release
    DISTRO=$ID
    LIKE=${ID_LIKE:-""}
else
    echo -e "${RED}Error: Cannot detect Linux distribution from /etc/os-release${NC}"
    exit 1
fi

echo -e "Detected: ${GREEN}$NAME ($DISTRO)${NC}"

# 1. Install System Dependencies & Tools
echo -e "\n${BLUE}[1/4] Installing development libraries, Vulkan, X11, Wayland, and ADB...${NC}"

case "$DISTRO" in
    ubuntu|debian|linuxmint|pop)
        sudo apt-get update
        sudo apt-get install -y --no-install-recommends \
            build-essential \
            pkg-config \
            libfontconfig1-dev \
            libfreetype6-dev \
            libasound2-dev \
            libx11-dev \
            libx11-xcb-dev \
            libxcb1-dev \
            libxcursor-dev \
            libxrandr-dev \
            libxi-dev \
            libxkbcommon-dev \
            libxkbcommon-x11-dev \
            libwayland-dev \
            libvulkan-dev \
            libssl-dev \
            v4l2loopback-dkms \
            v4l-utils \
            android-tools-adb
        ;;
    fedora|rhel|centos)
        sudo dnf install -y \
            gcc \
            gcc-c++ \
            pkgconf-pkg-config \
            fontconfig-devel \
            freetype-devel \
            alsa-lib-devel \
            libX11-devel \
            libXcursor-devel \
            libXrandr-devel \
            libXi-devel \
            libxkbcommon-devel \
            libxkbcommon-x11-devel \
            wayland-devel \
            vulkan-loader-devel \
            openssl-devel \
            v4l2loopback \
            v4l-utils \
            android-tools
        ;;
    arch|manjaro|endeavouros)
        sudo pacman -S --needed --noconfirm \
            base-devel \
            pkgconf \
            fontconfig \
            freetype2 \
            alsa-lib \
            libx11 \
            libxcursor \
            libxrandr \
            libxi \
            libxkbcommon \
            libxkbcommon-x11 \
            wayland \
            vulkan-icd-loader \
            openssl \
            v4l2loopback-dkms \
            v4l-utils \
            android-tools
        ;;
    *)
        echo -e "${YELLOW}Warning: Unknown distro $DISTRO (like: $LIKE). Please install packages manually:${NC}"
        echo "  - Fontconfig, ALSA, X11, Wayland, Vulkan, OpenSSL, v4l2loopback, ADB"
        ;;
esac

# 2. Configure Virtual Camera (v4l2loopback)
echo -e "\n${BLUE}[2/4] Setting up v4l2loopback virtual camera (/dev/video10)...${NC}"

# Module options:
# - devices=1: single camera node
# - video_nr=10: pins to /dev/video10
# - card_label="PhoneCam": names camera in Chrome, OBS, Meet, Zoom
# - exclusive_caps=1: prevents WebRTC browsers from confusing output with input
sudo modprobe -r v4l2loopback 2>/dev/null || true
sudo modprobe v4l2loopback devices=1 video_nr=10 card_label="PhoneCam" exclusive_caps=1

# Persist across reboots
echo "Persisting module load on boot..."
echo "v4l2loopback" | sudo tee /etc/modules-load.d/v4l2loopback.conf > /dev/null
echo 'options v4l2loopback devices=1 video_nr=10 card_label="PhoneCam" exclusive_caps=1' | sudo tee /etc/modprobe.d/v4l2loopback.conf > /dev/null

# 3. Configure udev rules for non-root camera and ADB access
echo -e "\n${BLUE}[3/4] Configuring non-root device permissions...${NC}"

# Add current user to video and plugdev groups
sudo usermod -aG video "$USER" || true
if getent group plugdev >/dev/null; then
    sudo usermod -aG plugdev "$USER" || true
fi

# Udev rule for /dev/video10
cat << 'EOF' | sudo tee /etc/udev/rules.d/99-phonecam-vcam.rules > /dev/null
KERNEL=="video10*", SUBSYSTEM=="video4linux", ATTR{name}=="PhoneCam*", MODE="0666", GROUP="video"
EOF

sudo udevadm control --reload-rules && sudo udevadm trigger

# 4. Status Verification
echo -e "\n${BLUE}[4/4] Verifying virtual camera status...${NC}"
if [ -e /dev/video10 ]; then
    echo -e "${GREEN}✓ Virtual camera device /dev/video10 is ready!${NC}"
    if command -v v4l2-ctl >/dev/null 2>&1; then
        v4l2-ctl --device=/dev/video10 --all | head -n 8 || true
    fi
else
    echo -e "${YELLOW}Notice: /dev/video10 was not found immediately. A reboot or 'sudo modprobe v4l2loopback' may be required.${NC}"
fi

echo -e "\n${GREEN}=== PhoneCam Linux Setup Completed Successfully! ===${NC}"
echo "You can now run PhoneCam via:"
echo "  cargo run -p phonecam-app"
echo "Or download the pre-compiled Linux binary from GitHub Releases."
