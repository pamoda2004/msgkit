package io.github.pamoda2004.msgkit.android

import android.system.Os
import android.system.OsConstants
import android.util.Log
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.pamoda2004.msgkit.CallOutcome
import io.github.pamoda2004.msgkit.Channel
import io.github.pamoda2004.msgkit.DeviceBatch
import io.github.pamoda2004.msgkit.DeviceOptions
import io.github.pamoda2004.msgkit.Direction
import io.github.pamoda2004.msgkit.Msgkit
import io.github.pamoda2004.msgkit.MsgkitException
import io.github.pamoda2004.msgkit.RawCall
import io.github.pamoda2004.msgkit.RawSms
import io.github.pamoda2004.msgkit.WhatsAppExportOptions
import io.github.pamoda2004.msgkit.instant
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import org.junit.runner.RunWith

/** Runs on a device/emulator: proves libmsgkit_ffi.so loads through JNA and behaves like the JVM build. */
@RunWith(AndroidJUnit4::class)
class MsgkitAndroidTest {
    @Test
    fun nativeLibraryLoadsAndReportsVersions() {
        val pageSize = Os.sysconf(OsConstants._SC_PAGESIZE)
        Log.i("msgkit", "device page size = $pageSize bytes")
        val info = Msgkit.version()
        assertEquals("1.0.0", info.schemaVersion)
        assertTrue(info.operations.contains("parse_whatsapp_export"))
    }

    @Test
    fun whatsAppExport() {
        val text =
            "31/12/2023, 21:41 - Alice: Happy new year!\n" +
                "31/12/2023, 21:42 - Bob: Same to you\n" +
                "01/01/2024, 00:10 - Missed voice call\n"
        val r = Msgkit.parseWhatsAppExport(text, WhatsAppExportOptions(selfName = "Alice", utcOffsetMinutes = 330))
        assertEquals(emptyList<String>(), r.warnings)
        assertEquals(2, r.messages.size)
        assertEquals("2023-12-31T16:11:00Z", r.messages[0].timestamp)
        assertEquals(Direction.OUTBOUND, r.messages[0].direction)
        assertEquals(Channel.WHATSAPP, r.messages[0].channel)
        assertEquals(CallOutcome.MISSED, r.calls.single().outcome)
    }

    @Test
    fun unicodeSurvivesTheRoundTrip() {
        val body = "සිංහල 🎉 café \"quoted\""
        val r = Msgkit.parseWhatsAppExport("01/02/2024, 10:00 - සමන්: $body\n")
        assertEquals(body, r.messages.single().body)
        assertEquals(
            "සමන්",
            r.messages
                .single()
                .sender.id,
        )
    }

    @Test
    fun deviceRowsAndInstantHelper() {
        val batch =
            DeviceBatch(
                sms = listOf(RawSms(address = "0771234567", body = "Thanks!", dateMs = 1704067260000, kind = 2)),
                calls = listOf(RawCall(number = "+94771234567", dateMs = 1704067400000, kind = 3)),
            )
        val r = Msgkit.normalizeDevice(batch, DeviceOptions(ownNumber = "+94710000000", defaultCountryCode = "94"))
        assertEquals(
            "+94771234567",
            r.messages
                .single()
                .recipients
                .single()
                .id,
        )
        assertEquals(CallOutcome.MISSED, r.calls.single().outcome)
        assertEquals(java.time.Instant.parse("2024-01-01T00:01:00Z"), r.messages.single().instant)
    }

    @Test
    fun webhookSignature() {
        // HMAC-SHA256("secret", "body")
        val mac = javax.crypto.Mac.getInstance("HmacSHA256")
        mac.init(javax.crypto.spec.SecretKeySpec("secret".toByteArray(), "HmacSHA256"))
        val sig = "sha256=" + mac.doFinal("body".toByteArray()).joinToString("") { "%02x".format(it) }
        assertTrue(Msgkit.verifySignature("secret", "body", sig))
        assertFalse(Msgkit.verifySignature("secret", "bodY", sig))
    }

    @Test
    fun errorsSurfaceAsExceptions() {
        try {
            Msgkit.parseFacebookExport("[1,2]")
            fail("expected MsgkitException")
        } catch (e: MsgkitException) {
            assertEquals("parse_error", e.code)
        }
    }

    @Test
    fun manyThreads() {
        val text = "31/12/2023, 21:41 - Alice: hi\n".repeat(200)
        val expected = Msgkit.parseWhatsAppExport(text)
        val threads =
            (1..4).map {
                Thread { repeat(10) { assertEquals(expected, Msgkit.parseWhatsAppExport(text)) } }.also { it.start() }
            }
        threads.forEach { it.join() }
    }
}
