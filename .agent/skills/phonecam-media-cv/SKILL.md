---
name: phonecam-media-cv
description: Technical and algorithmic guide for pure-Rust Computer Vision face detection, zero-allocation PTZ tracking, and 6 OBSBOT hand gestures.
---

# Pure-Rust Computer Vision & Gesture Recognition Skill Guide

This skill guide provides an exhaustive technical and algorithmic specification of the pure-Rust Computer Vision (CV) subsystem implemented in `phonecam-media`. It covers face detection, zero-allocation PTZ auto-framing tracking, the 6 OBSBOT-style hand gesture classifiers, mathematical formulations, debounce/cooldown state machines, and stage HUD integration.

---

## 1. Overview & Architectural Boundaries

The PhoneCam CV engine operates entirely in native Rust without dependencies on OpenCV, ONNX Runtime, Python, or external C/C++ libraries. It processes premultiplied BGRA video frames directly in the media pipeline at up to 60 FPS.

```
Incoming H.264 Bitstream
          │
          ▼
   openh264 Decoder
          │
          ▼
    YUV420P Frame
    ├──► NV12 Conversion ──► Shared Memory Framebus (C:\ProgramData\PhoneCam\framebuffer-v2.bin)
    └──► BGRA Conversion
              │
              ▼
    Pure-Rust CV Engine (phonecam-media)
    ├── 1. FaceDetector (YCbCr segmentation, Haar-like luma contrast, BFS clustering)
    │         │
    │         ▼
    │   AutoFramingTracker (1.5% deadband, exponential smoothing α = 0.12)
    │
    └── 2. GestureDetector (Face masking, valley depth checks, O(1) loop scanner)
              │
              ▼
    GPUI Application Runtime (phonecam-ui)
    ├── Double-buffered RenderImage recycling (window.drop_image)
    └── Stage HUD Gesture Chips & PTZ Viewport Overlay
```

---

## 2. Zero-Allocation Hot Path Design

To sustain 60 FPS (>124 MB/s pixel throughput) without garbage collection pauses or allocator lock contention, both `FaceDetector` and `GestureDetector` pre-allocate all internal memory at construction:

| Structure | Pre-Allocated Scratch Buffers | Sizing / Constraints |
| :--- | :--- | :--- |
| **Grid Dimensions** | `MAX_GRID_W = 160`, `MAX_GRID_H = 120` | Max 19,200 cells (`MAX_GRID_CELLS`) |
| **Masks & Luma** | `skin_mask`, `filtered_mask`, `luma` | `vec![0u8; MAX_GRID_CELLS]` |
| **BFS State** | `visited: Vec<bool>`, `queue: Vec<usize>` | `vec![false; MAX_GRID_CELLS]`, capacity `19,200` |
| **Projections** | `proj_x: [u16; 160]`, `first/last_skin_x/y` | Fixed-size stack arrays |
| **Candidates** | `candidates: Vec<CandidateRegion>` | Pre-allocated capacity `64` / `32` |

During per-frame processing, scratch buffers are reset in-place via `.fill()` or `.clear()`, ensuring **zero dynamic heap allocations** on the hot path.

---

## 3. Face Detection Pipeline

### 3.1 YCbCr BT.601 Skin Chrominance Segmentation
Every pixel sampled on the downscaled grid is converted from RGB to YCbCr using fixed-point integer arithmetic:

$$\begin{aligned}
Y  &= (77 \cdot R + 150 \cdot G + 29 \cdot B) \gg 8 \\
C_b &= 128 + (-43 \cdot R - 85 \cdot G + 128 \cdot B) \gg 8 \\
C_r &= 128 + (128 \cdot R - 107 \cdot G - 21 \cdot B) \gg 8
\end{aligned}$$

A cell is classified as human skin if and only if it satisfies all chrominance and photometric bounds:
- $75 \le C_b \le 132$
- $130 \le C_r \le 178$
- $R > G$ and $G \ge (B - 15)$
- $R > 40$ and $Y \ge 25$
- $|R - G| \ge 6$ (eliminates grayscale and neutral surfaces)

### 3.2 3x3 Morphological Noise Filtering
A local $3 \times 3$ kernel smooths the binary skin mask:
- Active skin cells survive if they have $\ge 3$ skin neighbors in their $3 \times 3$ neighborhood.
- Inactive cells are filled if they are surrounded by $\ge 6$ skin neighbors.
- Preserves genuine facial contours while eliminating scattered specular highlights and noise.

### 3.3 Connected Component Blob Extraction (BFS)
Contiguous skin cells are clustered using a 4-connected Breadth-First Search (BFS) queue.
- Blobs must meet minimum size thresholds: $\text{width}, \text{height} \ge \max(4, \min(W, H)/25)$ and $\text{pixel\_count} \ge 16$.
- Aspect ratio filter: $0.75 \le \frac{H}{W} \le 2.30$.
- Fill density filter: $0.28 \le \frac{\text{pixel\_count}}{W \cdot H} \le 0.96$.

### 3.4 Haar-Like Vertical Luma Contrast & Facial Verification
Human facial topology exhibits distinctive vertical luminance transitions due to orbital sockets, nasal bridges, and forehead highlights. Candidates are evaluated across 3 vertical zones:
1. **Forehead Zone**: $10\% \text{ to } 35\%$ of candidate height.
2. **Eye Sockets Zone**: $35\% \text{ to } 60\%$ of candidate height.
3. **Cheeks Zone**: $60\% \text{ to } 85\%$ of candidate height.

The eye contrast score evaluates the orbital recession darkness relative to forehead and cheekbones:

$$\text{eye\_contrast} = \max\left(0, \frac{\text{luma\_fh} + \text{luma\_ck}}{2} - \text{luma\_eye}\right)$$

### 3.5 Bilateral Symmetry & Composite Confidence
Candidates are scored across 5 geometric and photometric dimensions:

$$\begin{aligned}
S_{\text{aspect}} &= 1.0 - \min\left(1.0, \frac{|\text{aspect} - 1.35|}{1.0}\right) \\
S_{\text{fill}}   &= 1.0 - \min\left(1.0, \frac{|\text{fill} - 0.60|}{0.40}\right) \\
S_{\text{sym}}    &= 1.0 - \min\left(1.0, \frac{|\text{luma}_{\text{left}} - \text{luma}_{\text{right}}|}{80.0}\right) \\
S_{\text{size}}   &= \text{clamp}\left(\frac{\text{pixels}}{\text{cells}} \cdot 8.0, 0.2, 1.0\right) \\
S_{\text{contrast}} &= \text{clamp}\left(\frac{\text{eye\_contrast}}{15.0}, 0.0, 0.4\right)
\end{aligned}$$

$$\text{Confidence} = 0.30 \cdot S_{\text{aspect}} + 0.30 \cdot S_{\text{fill}} + 0.20 \cdot S_{\text{sym}} + 0.10 \cdot S_{\text{size}} + 0.10 \cdot S_{\text{contrast}}$$

Candidates with $\text{Confidence} \ge 0.28$ qualify. A proximity bonus ($+0.20$) is added when a candidate aligns within $0.25$ normalized distance from the previous locked face.

---

## 4. Zero-Allocation PTZ Face Tracking & Smoothing

`AutoFramingTracker` controls dynamic Pan-Tilt-Zoom (PTZ) digital crop windows:

### 4.1 1.5% Deadband Hysteresis
To prevent distracting jitter from micro-movements, breathing, or sensor noise:
- If $|\text{target\_cx} - \text{curr\_cx}| < 0.015$, horizontal tracking is held stationary.
- If $|\text{target\_cy} - \text{curr\_cy}| < 0.015$, vertical tracking is held stationary.
- Only deliberate displacements exceeding the 1.5% threshold trigger smooth camera motion.

### 4.2 Temporal Exponential Smoothing ($\alpha = 0.12$)
When the subject moves beyond the deadband, positions update smoothly via exponential moving average:

$$P_t = P_{t-1} \cdot (1 - \alpha) + P_{\text{target}} \cdot \alpha \quad (\alpha = 0.12)$$

This creates smooth, cinematic camera operator panning without robotic step-changes.

### 4.3 Sensor Boundary Clamping & Aspect Ratio Preservation
The crop window maintains the sensor aspect ratio (e.g. 16:9) and clamps coordinates to $[0.0, 1.0]$:

$$\text{crop\_x} = \text{clamp}(c_x - \frac{w_{\text{crop}}}{2}, 0.0, 1.0 - w_{\text{crop}})$$

### 4.4 Occlusion Bridging
If face detection fails temporarily (e.g. subject turns profile), the tracker holds the last valid position for up to **6 frames** (~100-200ms) with an exponential confidence decay factor ($0.85^k$).

---

## 5. The 6 OBSBOT Hand Gestures

`GestureDetector` identifies 6 distinct hand gestures for touch-free camera operation:

| Gesture | Icon | Primary Semantic Action | Trigger Conditions |
| :--- | :---: | :--- | :--- |
| **OpenPalm** | ✋ | Toggle tracking lock / pause | Aspect $0.65..1.65$, palm solidity $\ge 35\%$, $\ge 3$ spread finger peaks |
| **PeaceV** | ✌ | Toggle zoom (1.0x $\leftrightarrow$ 2.0x) | Aspect $1.05..3.50$, lower fill $\ge 25\%$, exactly 2 peaks with deep valley |
| **IndexPoint** | ☝ | Set zoom to 1.5x | Tall aspect $1.20..3.80$, single central peak (width $\le 65\%$), solid fist |
| **ShakaCall** | 🤙 | Reset framing (0.50, 0.50) & 1.0x zoom | Aspect $0.55..1.65$, 2 peaks at lateral edges (span $\ge 40\%$), folded center |
| **PinchOk** | 🤏 | Set macro zoom to 2.2x | Internal topological circular hole enclosed by skin loop |
| **ThumbsUp** | 👍 | Quick confirm / toggle zoom | Compact aspect $0.70..1.90$, lower fill $\ge 40\%$, 1 single lateral peak |

### 5.1 Face Area Masking
Before segmenting hand skin pixels, the detected face bounding box (expanded by a $4\%$ margin horizontally and $+8\%$ vertically for neck exclusion) is zeroed out in the skin mask. This prevents cheek or neck contours from interfering with hand clustering.

### 5.2 Valley Depth Checks ($\ge 25\%$ Dip)
Finger peaks in the upper projection array `proj_x` are filtered against false local ripples. Between two candidate finger peaks $P_a$ and $P_b$, the minimum projection valley $V_{\min}$ must dip by at least $25\%$:

$$V_{\min} \le \min(P_a, P_b) \cdot 0.75$$

If the valley dip is insufficient, peaks are merged into the dominant peak.

### 5.3 O(1) Enclosed Loop Scanner for Pinch/OK
The `PinchOk` gesture forms a closed circular loop between the thumb and index finger tip:
1. Row projections record `first_skin_x[y]` and `last_skin_x[y]` across the bounding box.
2. Column projections record `first_skin_y[x]` and `last_skin_y[x]`.
3. An interior non-skin cell $(x, y)$ is classified as an enclosed hole if:

$$\text{first\_skin\_x}[y] < x < \text{last\_skin\_x}[y] \quad \text{and} \quad \text{first\_skin\_y}[x] < y < \text{last\_skin\_y}[x]$$

Detecting $\ge 1$ enclosed loop cells with valid hand aspect confirms the circular pinch geometry.

### 5.4 Palm Base Solidity Ratio
Open palm and fist gestures evaluate the fill density of the lower half of the candidate region (`lower_fill`):

$$\text{lower\_fill} = \frac{\text{pixels in lower half}}{W \cdot (H / 2)} \ge 0.35$$

This ensures a solid palm or wrist base is present, distinguishing real hands from loose environmental artifacts.

---

## 6. Debounce & Refractory Cooldown State Machine

To prevent accidental triggers and repeated action cycling, gesture events pass through a two-stage temporal filter:

```
[ New Frame Analysis ]
          │
          ├── Confidence >= 0.30 and Candidate matches current?
          │         ├── Yes ──► consecutive_count += 1
          │         └── No  ──► current_candidate = Candidate, consecutive_count = 1
          │
          ▼
[ Trigger Check ]
          ├── consecutive_count >= 3 AND cooldown_remaining == 0 ?
          │         ├── TRUE  ──► triggered = true (Rising Edge Trigger!)
          │         │             cooldown_remaining = 45 frames
          │         └── FALSE ──► triggered = false
          │
          ▼
[ Cooldown Tick ]
          └── If cooldown_remaining > 0: cooldown_remaining -= 1
```

- **3-Frame Debounce**: Requires 3 consecutive identical frames ($50-100\text{ms}$) to confirm the gesture.
- **45-Frame Refractory Cooldown**: Enforces a $45\text{-frame}$ lockout ($\sim 0.75\text{s}$ at 60 FPS, $1.5\text{s}$ at 30 FPS) after trigger confirmation. While cooling down, the gesture may still be tracked visually, but `triggered` remains false.

---

## 7. Stage HUD Chips & Visual Feedback

When a gesture triggers:
1. `GestureEvent { gesture, confidence, center_x, center_y, triggered: true }` is dispatched to GPUI.
2. The UI preview stage renders a floating HUD chip displaying the gesture icon (e.g. `✌ 2.0x Zoom`) with smooth fade-in and scale animations.
3. The live action (zoom step, tracking lock toggle, or framing reset) executes seamlessly without interrupting active video streaming.

---

## 8. Diagnostic & Verification Procedures

### Running Media & CV Unit Tests
```powershell
cargo test -p phonecam-media
```
Key tests covering CV algorithms:
- `face_detect::tests::skin_chrominance_detection`
- `face_detect::tests::face_detector_empty_or_zero_frame_returns_none`
- `face_detect::tests::face_detector_detects_synthetic_face`
- `face_detect::tests::face_detector_zero_allocation_hot_path`
- `gesture::tests::gesture_detector_detects_open_palm`
- `gesture::tests::gesture_detector_detects_peace_v_sign`
- `gesture::tests::gesture_detector_detects_pinch_ok_sign`
- `gesture::tests::gesture_detector_detects_shaka_call`
- `gesture::tests::gesture_detector_detects_index_pointing_up`
- `gesture::tests::gesture_detector_detects_thumbs_up`
- `gesture::tests::gesture_detector_cooldown_prevents_rapid_fire`
- `gesture::tests::gesture_detector_zero_allocation_hot_path`

### Verifying Hot-Path Zero Allocations
Inspect `crates/phonecam-media/src/face_detect.rs` and `gesture.rs`:
- No `Vec::new()`, `format!()`, or unbounded allocation routines inside `detect()` or `average_luma()`.
- Pre-allocated arrays and vectors are sized to `MAX_GRID_CELLS` at initialization.
