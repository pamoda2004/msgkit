// A tiny application that uses msgkit as an *installed dependency* (resolved from a Maven repository).
// Prints chat statistics; the same program exists for Node.js, Python and Rust and prints the same output.
import dev.msgkit.CallOutcome
import dev.msgkit.Msgkit
import dev.msgkit.WhatsAppExportOptions
import java.io.File
import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec

fun main(args: Array<String>) {
    val path = args.firstOrNull() ?: "../../../crates/msgkit-core/tests/fixtures/whatsapp_android_dmy.txt"
    val report = Msgkit.parseWhatsAppExport(
        File(path).readText(Charsets.UTF_8),
        WhatsAppExportOptions(selfName = "Alice", utcOffsetMinutes = 330),
    )

    val messages = report.messages
    val calls = report.calls
    println("events: ${report.events.size}")
    println("messages: ${messages.size}, calls: ${calls.size}")
    println("per sender:")
    for ((name, n) in messages.groupingBy { it.sender.id }.eachCount().toSortedMap()) println("  $name: $n")
    println("attachments: ${messages.sumOf { it.attachments.size }}")
    println("missed calls: ${calls.count { it.outcome == CallOutcome.MISSED }}")
    println("first: ${report.events.first().timestamp}  last: ${report.events.last().timestamp}")

    // Webhook signature check with a signature computed independently by the language's own crypto library.
    val body = """{"object":"page","entry":[]}"""
    val mac = Mac.getInstance("HmacSHA256").apply { init(SecretKeySpec("app-secret".toByteArray(), "HmacSHA256")) }
    val good = "sha256=" + mac.doFinal(body.toByteArray()).joinToString("") { "%02x".format(it) }
    println("signature valid: ${Msgkit.verifySignature("app-secret", body, good)}")
    println("tampered body valid: ${Msgkit.verifySignature("app-secret", "$body ", good)}")
}
