# Rombak UI: apa yang diubah, dan apa yang sengaja ditolak

Saya mengkaji dua skill dari `Leonxlnx/taste-skill` (99k stars, repo asli —
terverifikasi lewat GitHub API): **redesign-existing-projects** dan
**minimalist-ui**. Keduanya berguna, tapi keduanya ditulis untuk **halaman web
dan marketing**, bukan untuk panel instrumen desktop. Menerapkannya mentah-mentah
akan merusak produk ini, jadi di bawah ini saya pisahkan mana yang saya ambil,
mana yang saya tolak, dan alasannya.

## Audit: apa yang benar-benar slop

Sebelum menyentuh apa pun, saya periksa kodenya:

    hover state    : TIDAK ADA (nol di seluruh UI)
    focus ring     : TIDAK ADA
    active/pressed : TIDAK ADA
    tabular figures: TIDAK ADA

Ini temuan yang paling serius dan paling tidak terlihat di screenshot: **tidak
ada satu pun kontrol yang merespons pointer**. Tombol, tab, toggle, tile kamera —
semuanya diam saat disentuh. Itulah kenapa UI-nya terasa seperti mockup, bukan
aplikasi. Skill benar soal ini dan saya perbaiki.

## Yang saya ambil

### 1. Hover, press, dan state disabled (temuan utama)

`Button` sekarang punya hover per varian, press feedback, dan cursor yang benar.
Tab rail yang tidak aktif menyala saat hover. Tombol disabled **tidak** menyala —
kontrol yang mati tapi bereaksi saat hover menjanjikan aksi yang tidak akan
terjadi.

Varian punya target hover berbeda, sehingga hover benar-benar membedakan, bukan
sekadar dekorasi. Ini dipin oleh test.

### 2. Tabular figures untuk semua angka

Skill menandai ini eksplisit dan benar: setiap readout di jendela ini berubah
sekitar sekali per detik — fps, bitrate, latensi, dropped percent, hitungan
mundur pairing. Dengan digit proporsional, seluruh baris bergeser setiap update,
dan itu terbaca sebagai **pengukuran yang tidak stabil**, bukan sebagai teks yang
berubah. `numeric_font()` mengaktifkan fitur OpenType `tnum`.

### 3. Panel dirampingkan

Panel sebelumnya menumpuk border + fill + shadow sekaligus — itu "card look"
generik yang skill sebut. Sekarang panel adalah **fill saja**: pemisahnya dari
shell datang dari warna, dan aturan di header melakukan sisanya. Kedalaman di
jendela ini datang dari stage yang recessed, bukan dari setiap panel mengklaim
melayang.

Info box dan baris USB yang berada di dalam panel juga kehilangan bordernya —
mereka punya batas ganda dengan panel induknya.

## Yang saya tolak, dan kenapa

**Palet terang / warm monochrome (`#F7F6F3`, `#111111`).**
`minimalist-ui` menetapkan kanvas putih tulang. Itu benar untuk dokumen dan
halaman marketing; **salah untuk alat broadcast**. Jendela ini duduk di samping
OBS dan Meet, sering di ruangan gelap, dan stage preview harus hitam karena itu
letterbox di sekitar gambar kamera. Panel putih akan memantulkan cahaya ke wajah
operator dan menenggelamkan gambar. Palet gelap dipertahankan.

**Font editorial serif (Playfair, Newsreader, Instrument Serif).**
Untuk judul hero marketing, bagus. Untuk label kontrol 13px di panel 396px, serif
editorial adalah pilihan yang salah: keterbacaan turun dan tidak ada yang dibaca
sebagai "editorial" pada ukuran itu. Tiga muka yang ada — Segoe UI Variable Text,
Bahnschrift, Cascadia Mono — sudah dipilih berdasarkan pekerjaan, semuanya ship
dengan Windows 11, jadi tidak ada font yang perlu diunduh dan tidak ada flash.

**"Bento grid", `py-24`/`py-32`, animasi scroll-entry.**
Ini kosakata halaman marketing. Jendela ini adalah panel instrumen: tidak ada
scroll sehalaman, tidak ada hero, dan whitespace besar justru membuang ruang yang
dibutuhkan kontrol. Spasi di sini ditentukan oleh kepadatan informasi, bukan oleh
nafas halaman.

**Gradient, glow, glassmorphism, grain overlay.**
`redesign-existing-projects` menyarankan tekstur dan gradient ambient untuk
mengisi bagian yang "kosong dan datar". Di panel instrumen, setiap piksel yang
tidak membawa informasi bersaing dengan gambar kamera. Aturan asli proyek ini
sudah benar: satu aksen, dan aksen berarti "bisa diklik", bukan hiasan.

**Ikon Phosphor menggantikan Lucide.**
Tidak ada ikon di UI ini sama sekali — tidak ada yang perlu diganti.

## Yang belum dikerjakan

**Focus ring.** Skill menandainya sebagai persyaratan aksesibilitas, dan itu
benar. GPUI punya `track_focus`, tapi jendela ini belum punya navigasi keyboard
sama sekali: tidak ada Tab order, dan shortcut Space/F11/Ctrl+1..4 yang
didokumentasikan di panel Preferences **belum terpasang** (`register_shortcuts`
masih kosong). Menambahkan cincin fokus tanpa navigasi keyboard akan menjadi
dekorasi yang menyesatkan. Ini pekerjaan tersendiri, dan saya catat sebagai
belum selesai daripada berpura-pura sudah.

**Verifikasi visual.** Sesi ini tidak bisa menangkap window DirectX-composited
(capture mengembalikan permukaan kosong), jadi perubahan ini diverifikasi lewat
kode, test, dan aplikasi yang berjalan bersih — bukan dengan mata.
