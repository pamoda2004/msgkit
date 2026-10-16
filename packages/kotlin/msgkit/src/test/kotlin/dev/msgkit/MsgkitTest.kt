package dev.msgkit

import java.io.File
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec
import kotlin.test.Test
import kotlin.test.assertContentEquals
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertNotEquals
import kotlin.test.assertTrue

private fun fixture(name: String): String = File(System.getProperty("msgkit.fixtures"), name).readText(Charsets.UTF_8)

private fun hmacHex(
    secret: String,
    body: String,
): String {
    val mac = Mac.getInstance("HmacSHA256")
    mac.init(SecretKeySpec(secret.toByteArray(), "HmacSHA256"))
    return mac.doFinal(body.toByteArray()).joinToString("") { "%02x".format(it) }
}

class MsgkitTest {
    @Test
    fun `versions agree with the Rust workspace`() {
        val info = Msgkit.version()
        assertEquals("1.0.0", info.schemaVersion)
        assertEquals(System.getProperty("msgkit.workspaceVersion"), info.crateVersion)
        assertTrue("parse_whatsapp_export" in info.operations)
    }

    @Test
    fun `WhatsApp export Android day-first`() {
        val r = Msgkit.parseWhatsAppExport(fixture("whatsapp_android_dmy.txt"))
        assertEquals(emptyList(), r.warnings)
        assertEquals(8, r.events.size)
        assertEquals(7, r.messages.size)
        assertEquals(CallOutcome.MISSED, r.calls.single().outcome)
        assertEquals("2023-12-31T21:41:00Z", r.events[0].timestamp)
        assertEquals(Channel.WHATSAPP, r.events[0].channel)

        val oks = r.messages.filter { it.body == "ok" }
        assertEquals(2, oks.size)
        assertNotEquals(oks[0].id, oks[1].id)
    }

    @Test
    fun `options are applied`() {
        val r =
            Msgkit.parseWhatsAppExport(
                fixture("whatsapp_android_dmy.txt"),
                WhatsAppExportOptions(selfName = "alice", utcOffsetMinutes = 330),
            )
        assertEquals("2023-12-31T16:11:00Z", r.events[0].timestamp)
        assertEquals(Direction.OUTBOUND, r.events[0].direction)
        assertEquals(Direction.INBOUND, r.events[1].direction)
    }

    @Test
    fun `ambiguous dates warn and can be overridden`() {
        val text = "03/04/2024, 10:00 - A: x\n"
        val auto = Msgkit.parseWhatsAppExport(text)
        assertTrue(auto.warnings.any { "ambiguous" in it })
        assertEquals("2024-04-03T10:00:00Z", auto.events[0].timestamp)

        val mdy = Msgkit.parseWhatsAppExport(text, WhatsAppExportOptions(dateOrder = DateOrder.MDY))
        assertEquals(emptyList(), mdy.warnings)
        assertEquals("2024-03-04T10:00:00Z", mdy.events[0].timestamp)
    }

    @Test
    fun `Unicode survives the round trip regardless of the platform charset`() {
        val body = "සිංහල 🎉 café \"quoted\" \\ back\\slash"
        val r = Msgkit.parseWhatsAppExport("01/02/2024, 10:00 - සමන්: $body\n")
        val m = r.messages.single()
        assertEquals(body, m.body)
        assertEquals("සමන්", m.sender.id)
    }

    @Test
    fun `instant helper parses timestamps`() {
        val r = Msgkit.parseWhatsAppExport("01/02/2024, 10:00 - A: hi\n")
        assertEquals(java.time.Instant.parse("2024-02-01T10:00:00Z"), r.events[0].instant)
    }

    @Test
    fun `Facebook export`() {
        val r =
            Msgkit.parseFacebookExport(
                fixture("facebook_thread.json"),
                FacebookExportOptions(selfName = "Alice Perera"),
            )
        assertEquals(5, r.events.size)
        assertEquals("café ❤", r.messages[2].body)
        assertEquals(Direction.OUTBOUND, r.messages[2].direction)
        assertEquals(125, r.calls[1].durationSecs)
        assertTrue(r.calls[0].video)
    }

    @Test
    fun `device batch`() {
        val batch =
            DeviceBatch(
                sms =
                    listOf(
                        RawSms(
                            address = "+94771234567",
                            body = "Your code is 4821",
                            dateMs = 1704067200000,
                            kind = 1,
                            threadId = "5",
                        ),
                        RawSms(address = "0771234567", body = "Thanks!", dateMs = 1704067260000, kind = 2),
                    ),
                calls =
                    listOf(
                        RawCall(
                            number = "+94771234567",
                            dateMs = 1704067400000,
                            kind = 1,
                            durationSecs = 60,
                            name = "Kamal",
                        ),
                        RawCall(number = "0771234567", dateMs = 1704067500000, kind = 3),
                    ),
            )
        val r = Msgkit.normalizeDevice(batch, DeviceOptions(ownNumber = "+94710000000", defaultCountryCode = "94"))
        assertEquals(2, r.messages.size)
        assertEquals(
            "+94771234567",
            r.messages[1]
                .recipients
                .single()
                .id,
        )
        assertEquals(Direction.OUTBOUND, r.messages[1].direction)
        assertEquals(listOf(CallOutcome.ANSWERED, CallOutcome.MISSED), r.calls.map { it.outcome })
        assertEquals(
            "Kamal",
            r.calls[0]
                .participants
                .single()
                .displayName,
        )
        assertTrue(r.calls.all { it.channel == Channel.CELLULAR })
    }

    @Test
    fun `webhooks`() {
        val wa = Msgkit.normalizeWhatsAppWebhook(fixture("whatsapp_webhook.json"))
        assertEquals(3, wa.messages.size)
        assertEquals("wamid.AAA", wa.messages[0].externalId)
        assertEquals("Kamal", wa.messages[0].sender.displayName)
        assertEquals(
            "MEDIA1",
            wa.messages[1]
                .attachments
                .single()
                .sourceId,
        )
        assertTrue(wa.messages[0].raw != null)

        val ms = Msgkit.normalizeMessengerWebhook(fixture("messenger_webhook.json"))
        assertEquals(listOf(Direction.INBOUND, Direction.OUTBOUND), ms.messages.map { it.direction })
    }

    @Test
    fun `signature verification matches javax crypto`() {
        val body = fixture("whatsapp_webhook.json")
        val sig = "sha256=" + hmacHex("app-secret", body)
        assertTrue(Msgkit.verifySignature("app-secret", body, sig))
        assertTrue(!Msgkit.verifySignature("wrong", body, sig))
        assertTrue(!Msgkit.verifySignature("app-secret", "$body ", sig))
        assertTrue(!Msgkit.verifySignature("app-secret", body, "sha256=zz"))
    }

    @Test
    fun `errors are MsgkitException with stable codes`() {
        assertEquals("parse_error", assertFailsWith<MsgkitException> { Msgkit.parseFacebookExport("[1,2]") }.code)
        assertEquals("invalid_input", assertFailsWith<MsgkitException> { Msgkit.parseFacebookExport("not json") }.code)
        assertEquals(
            "parse_error",
            assertFailsWith<MsgkitException> {
                Msgkit.normalizeMessengerWebhook("""{"object":"whatsapp_business_account"}""")
            }.code,
        )
    }

    @Test
    fun `raw dispatch returns the envelope`() {
        val env = Msgkit.dispatchRaw("nope", "{}")
        assertTrue("\"ok\":false" in env && "unknown_op" in env, env)
    }

    @Test
    fun `bad records become warnings not exceptions`() {
        val r = Msgkit.parseWhatsAppExport("this is not a chat export")
        assertEquals(emptyList(), r.events)
        assertTrue(r.warnings.isNotEmpty())
    }

    @Test
    fun `re-import gives identical results`() {
        val text = fixture("whatsapp_android_dmy.txt")
        assertEquals(Msgkit.parseWhatsAppExport(text), Msgkit.parseWhatsAppExport(text))
    }

    @Test
    fun `large input`() {
        val line = "31/12/2023, 21:41 - Alice: some reasonably long message body to make the file big\n"
        val r = Msgkit.parseWhatsAppExport(line.repeat(50_000))
        assertEquals(50_000, r.events.size)
        assertEquals(
            50_000,
            r.events
                .map { it.id }
                .toSet()
                .size,
        )
    }

    @Test
    fun `threads share the library safely`() {
        val text = fixture("whatsapp_android_dmy.txt")
        val expected = Msgkit.parseWhatsAppExport(text)
        val pool = Executors.newFixedThreadPool(8)
        try {
            val futures =
                (1..8).map {
                    pool.submit<Boolean> { (1..100).all { Msgkit.parseWhatsAppExport(text) == expected } }
                }
            assertTrue(futures.all { it.get(60, TimeUnit.SECONDS) })
        } finally {
            pool.shutdownNow()
        }
    }

    @Test
    fun `typed decoding of a minimal export`() {
        val r = Msgkit.parseWhatsAppExport("01/02/2024, 10:00 - A: hi\n")
        assertContentEquals(listOf("hi"), r.messages.map { it.body })
    }
}
