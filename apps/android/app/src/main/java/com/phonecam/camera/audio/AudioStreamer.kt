package com.phonecam.camera.audio

import android.annotation.SuppressLint
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.MediaRecorder
import android.util.Log
import com.phonecam.camera.data.IdentityStore
import com.phonecam.camera.protocol.ChannelRole
import com.phonecam.camera.transport.DialResult
import com.phonecam.camera.transport.PeerConnection
import java.io.IOException
import java.util.concurrent.atomic.AtomicBoolean

/**
 * Streams real microphone PCM audio from Android to Desktop over a dedicated TCP connection.
 *
 * Multiplexed alongside video and control via [ChannelRole.AUDIO] (wire role 3).
 * Captures 16-bit mono PCM at 48000Hz (or 16000Hz fallback) using [AudioRecord],
 * transmitting framed audio sample chunks continuously with low latency.
 */
class AudioStreamer(
    private val identity: IdentityStore,
) {
    private val running = AtomicBoolean(false)
    private var workerThread: Thread? = null
    private var activeConnection: PeerConnection? = null
    private var activeRecord: AudioRecord? = null

    val isStreaming: Boolean
        get() = running.get()

    /**
     * Starts audio streaming to the desktop at [host]:[port].
     */
    @Synchronized
    fun start(host: String, port: Int) {
        if (running.get()) {
            Log.d(TAG, "AudioStreamer already running")
            return
        }
        running.set(true)

        val thread = Thread({
            runAudioLoop(host, port)
        }, "PhoneCam-AudioStreamer")
        thread.isDaemon = true
        workerThread = thread
        thread.start()
    }

    /**
     * Gracefully stops audio streaming, closing the TCP connection and releasing the [AudioRecord].
     */
    @Synchronized
    fun stop() {
        if (!running.getAndSet(false)) {
            return
        }
        Log.i(TAG, "Stopping audio streamer...")
        runCatching { activeConnection?.close() }
        runCatching {
            activeRecord?.stop()
            activeRecord?.release()
        }
        activeConnection = null
        activeRecord = null
        workerThread?.interrupt()
        workerThread = null
    }

    @SuppressLint("MissingPermission")
    private fun runAudioLoop(host: String, port: Int) {
        Log.i(TAG, "Connecting audio channel to $host:$port (role=AUDIO)...")
        val dialResult = PeerConnection.dialAsKnownDevice(
            identity = identity,
            host = host,
            port = port,
            role = ChannelRole.AUDIO,
        )

        if (dialResult !is DialResult.Ok) {
            Log.w(TAG, "Audio channel dial failed: $dialResult")
            running.set(false)
            return
        }

        val connection = dialResult.connection
        activeConnection = connection

        if (!running.get()) {
            connection.close()
            return
        }

        var audioRecord: AudioRecord? = null
        try {
            var sampleRate = 48000
            var minBuf = AudioRecord.getMinBufferSize(
                sampleRate,
                AudioFormat.CHANNEL_IN_MONO,
                AudioFormat.ENCODING_PCM_16BIT,
            )
            if (minBuf <= 0) {
                sampleRate = 16000
                minBuf = AudioRecord.getMinBufferSize(
                    sampleRate,
                    AudioFormat.CHANNEL_IN_MONO,
                    AudioFormat.ENCODING_PCM_16BIT,
                )
            }
            val recordBufferSize = maxOf(minBuf * 2, 4096)
            val record = AudioRecord(
                MediaRecorder.AudioSource.MIC,
                sampleRate,
                AudioFormat.CHANNEL_IN_MONO,
                AudioFormat.ENCODING_PCM_16BIT,
                recordBufferSize,
            )

            if (record.state != AudioRecord.STATE_INITIALIZED) {
                Log.w(TAG, "AudioRecord failed to initialize (state=${record.state})")
                record.release()
                return
            }

            record.startRecording()
            audioRecord = record
            activeRecord = record
            Log.i(TAG, "AudioRecord started at ${sampleRate}Hz mono 16-bit PCM")

            // Buffer pool: reuse pooled ByteArray to prevent per-chunk allocations on hot audio path
            val chunkBuffer = ByteArray(CHUNK_SIZE)

            while (running.get() && connection.isOpen) {
                val bytesRead = record.read(chunkBuffer, 0, chunkBuffer.size)
                if (bytesRead > 0) {
                    try {
                        connection.output.write(chunkBuffer, 0, bytesRead)
                        connection.output.flush()
                    } catch (e: IOException) {
                        Log.i(TAG, "Audio socket write ended: ${e.message}")
                        break
                    }
                } else if (bytesRead < 0) {
                    Log.w(TAG, "AudioRecord read returned error code: $bytesRead")
                    break
                }
            }
        } catch (t: Throwable) {
            Log.w(TAG, "Audio recording loop error: ${t.javaClass.simpleName}: ${t.message}")
        } finally {
            runCatching {
                audioRecord?.stop()
                audioRecord?.release()
            }
            runCatching {
                connection.close()
            }
            activeRecord = null
            activeConnection = null
            running.set(false)
            Log.i(TAG, "Audio channel closed and resources released")
        }
    }

    companion object {
        private const val TAG = "AudioStreamer"
        /** 2048 bytes = 1024 16-bit mono samples = ~21.3 ms of audio at 48kHz. */
        private const val CHUNK_SIZE = 2048
    }
}
