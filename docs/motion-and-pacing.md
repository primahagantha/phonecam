# Gerakan cepat, patah-patah, dan freeze

Catatan diagnosis untuk keluhan "kalau ada gerakan kencang, kamera lag / patah-patah".
Semua angka di sini **terukur**, bukan perkiraan.

## Cara mengukurnya

Mengukur "patah-patah" dengan polling tidak bisa dipercaya: timer Windows
terkuantisasi ~15.6 ms, jadi setiap gap terlihat sebagai kelipatan angka itu dan
stream mulus tidak bisa dibedakan dari yang tersendat. Pengukuran yang dipakai:

- **Interval antar frame dicatat di produsen** (`frame_intervals_us`), bukan oleh
  konsumen yang mem-poll.
- **Tiap tahap di video loop diberi waktu sendiri**: socket read, decode, publish.
- **Benchmark terisolasi** untuk jalur publish (`cargo run --release -p
  phonecam-media --example publish_bench`), karena mesin uji ini terlalu bising
  untuk mengukur end-to-end (binary yang sama menghasilkan 15.7, 30.0 dan 29.8
  fps pada tiga run berurutan).

## Ke mana waktu per frame pergi (1280x720, buffer 1920x1080)

Dari benchmark terisolasi, 200 iterasi:

    allocate-every-frame    8.06 ms/frame
    reuse-buffers           6.75 ms/frame
      scale 720p -> 1080p   3.75 ms   (56%)
      write_bgra            2.73 ms   (40%)
      write_nv12            0.05 ms

Dari sesi HP sungguhan (Xiaomi 2201117TG, Wi-Fi):

    socket read : mean 4.4 - 25.3 ms, worst 640 - 671 ms
    decode      : mean 5.0 - 17.0 ms
    publish     : mean 7.1 - 27.3 ms

## Tiga penyebab, dan statusnya

### 1. Alokasi per frame — sudah diperbaiki

Versi pertama mengalokasi empat buffer setiap frame (NV12, salinan I420, NV12
hasil scale, dan `bgra.clone()`): **11.3 MB per frame, ~338 MB/s pada 30 fps**.
Biaya alokasi dan memset membengkak tepat saat gerakan cepat membuat frame lebih
besar. Core C++ punya `scaledFrame_` yang dipakai ulang persis untuk ini; port
Rust-nya menghilangkannya. Sekarang semua buffer dipakai ulang lewat
`PublishScratch`.

### 2. Upscale yang sia-sia — sudah diperbaiki

Default output 1920x1080 sementara HP mengirim 1280x720, jadi setiap frame
di-upscale: **3.75 ms dari 6.75 ms biaya publish**, untuk menambah nol detail,
dan hasil yang dipublikasikan justru lebih buram daripada yang diterima.
Sekarang buffer mengikuti ukuran stream (`output_follows_stream`, default aktif),
sehingga kasus umum menjadi salinan langsung. Ukuran tetap bisa dipatok kalau
memang diinginkan.

### 3. Encoder VBR — di sisi Android, belum bisa diperbaiki dari sini

APK yang terpasang di HP bertanggal 10:18, sedangkan perbaikan CBR
(`fix(android): encode at a constant bitrate`) commit jam 10:23. Jadi HP masih
**VBR**. Buktinya terukur: bitrate naik dari 444 kbps menjadi 505 kbps saat
kamera digerakkan. Dengan VBR, gerakan cepat membuat encoder meledakkan bitrate
tepat saat link dan decoder paling sibuk.

Ini penyebab yang paling cocok dengan keluhan aslinya, dan perbaikannya ada di
aplikasi Android — bukan di port Rust.

## Yang bukan penyebabnya

Pipeline Rust **tidak** backlog. Kalau decoder tertinggal, socket read akan ~0 ms
(data sudah menumpuk) dan interval memanjang. Yang terukur sebaliknya: read
mayoritas idle menunggu frame berikutnya, p50 interval 33 ms (persis 30 fps),
decode 5 ms. Decoder dan konversi warna sehat.

## Catatan lingkungan

Mesin uji ini menjalankan VMware, beberapa harness, dan 341 proses; beban CPU
40% dan commit memori 30.9/38.7 GB. Variasi antar run karena itu besar, dan
angka di atas harus dibaca sebagai rentang, bukan nilai tunggal. Untuk regresi
yang bisa dipercaya, pakai benchmark terisolasi.
