package dev.msgkit

import dev.msgkit.internal.NativeBridge
import kotlinx.serialization.KSerializer
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

/**
 * Entry point of the msgkit Kotlin API. Thread-safe.
 *
 * ```kotlin
 * val report = Msgkit.parseWhatsAppExport(
 *     File("_chat.txt").readText(),
 *     WhatsAppExportOptions(selfName = "Alice", utcOffsetMinutes = 330),
 * )
 * report.messages.forEach { println("${it.timestamp} ${it.body}") }
 * ```
 *
 * Failures throw [MsgkitException]. Bad *records* inside an otherwise valid
 * input never throw; they are reported in [ImportReport.warnings].
 */
public object Msgkit {
    private val json = Json {
        ignoreUnknownKeys = true // newer library versions may add fields
        explicitNulls = false // omit nulls: the native side treats absent as default
        classDiscriminator = "type"
    }

    @Serializable
    private class WhatsAppInput(val text: String, val options: WhatsAppExportOptions)

    @Serializable
    private class FacebookInput(val thread: JsonElement, val options: FacebookExportOptions)

    @Serializable
    private class DeviceInput(val batch: DeviceBatch, val options: DeviceOptions)

    @Serializable
    private class WebhookInput(val payload: JsonElement)

    @Serializable
    private class SignatureInput(val secret: String, val body: String, val signature: String)

    /** Versions of the native library and of the JSON data contract. */
    @JvmStatic
    public fun version(): VersionInfo = call("version", "{}", VersionInfo.serializer())

    /**
     * Parses a WhatsApp "Export chat" text file.
     *
     * @param text the file contents (UTF-8)
     */
    @JvmStatic
    @JvmOverloads
    public fun parseWhatsAppExport(
        text: String,
        options: WhatsAppExportOptions = WhatsAppExportOptions(),
    ): ImportReport {
        val input = json.encodeToString(WhatsAppInput.serializer(), WhatsAppInput(text, options))
        return call("parse_whatsapp_export", input, ImportReport.serializer())
    }

    /**
     * Parses one Messenger `message_N.json` file ("Download Your Information").
     *
     * @param threadJson the file contents
     */
    @JvmStatic
    @JvmOverloads
    public fun parseFacebookExport(
        threadJson: String,
        options: FacebookExportOptions = FacebookExportOptions(),
    ): ImportReport {
        val thread = parseJson(threadJson, "threadJson")
        val input = json.encodeToString(FacebookInput.serializer(), FacebookInput(thread, options))
        return call("parse_facebook_export", input, ImportReport.serializer())
    }

    /** Normalizes Android SMS and call-log rows collected on a device. */
    @JvmStatic
    @JvmOverloads
    public fun normalizeDevice(
        batch: DeviceBatch,
        options: DeviceOptions = DeviceOptions(),
    ): ImportReport {
        val input = json.encodeToString(DeviceInput.serializer(), DeviceInput(batch, options))
        return call("normalize_device", input, ImportReport.serializer())
    }

    /**
     * Normalizes a WhatsApp Business Cloud API webhook body.
     *
     * Does **not** verify the signature: call [verifySignature] on the same raw
     * body first.
     */
    @JvmStatic
    public fun normalizeWhatsAppWebhook(rawBody: String): ImportReport =
        webhook("normalize_whatsapp_webhook", rawBody)

    /**
     * Normalizes a Messenger Platform webhook body.
     *
     * Does **not** verify the signature: call [verifySignature] on the same raw
     * body first.
     */
    @JvmStatic
    public fun normalizeMessengerWebhook(rawBody: String): ImportReport =
        webhook("normalize_messenger_webhook", rawBody)

    /**
     * Checks an `X-Hub-Signature-256` header against the **raw, unparsed**
     * request body (HMAC-SHA256 with the app secret, compared in constant time).
     */
    @JvmStatic
    public fun verifySignature(secret: String, body: String, signature: String): Boolean {
        val input = json.encodeToString(SignatureInput.serializer(), SignatureInput(secret, body, signature))
        return data("verify_signature", input).jsonObject.getValue("valid").jsonPrimitive.boolean
    }

    /**
     * Low-level access to any operation: sends [inputJson] and returns the raw
     * JSON response envelope (`{"ok":true,"data":...}` or `{"ok":false,"error":...}`).
     */
    @JvmStatic
    public fun dispatchRaw(op: String, inputJson: String): String = NativeBridge.call(op, inputJson)

    private fun webhook(op: String, rawBody: String): ImportReport {
        val payload = parseJson(rawBody, "rawBody")
        val input = json.encodeToString(WebhookInput.serializer(), WebhookInput(payload))
        return call(op, input, ImportReport.serializer())
    }

    private fun parseJson(text: String, argument: String): JsonElement =
        try {
            json.parseToJsonElement(text)
        } catch (e: kotlinx.serialization.SerializationException) {
            throw MsgkitException("invalid_input", "$argument is not valid JSON: ${e.message}")
        }

    private fun data(op: String, inputJson: String): JsonElement {
        val envelope = json.parseToJsonElement(NativeBridge.call(op, inputJson)).jsonObject
        if (!envelope.getValue("ok").jsonPrimitive.boolean) {
            val error = envelope.getValue("error").jsonObject
            throw MsgkitException(
                code = error.getValue("code").jsonPrimitive.content,
                message = error.getValue("message").jsonPrimitive.content,
            )
        }
        return envelope.getValue("data")
    }

    private fun <T> call(op: String, inputJson: String, serializer: KSerializer<T>): T =
        json.decodeFromJsonElement(serializer, data(op, inputJson))
}
