package io.github.pamoda2004.msgkit

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonObject
import java.io.File
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

/**
 * Runs the language-independent conformance suite (`conformance/`) through the Kotlin binding. The expected
 * files hold the canonical response envelope; every binding must reproduce them byte for byte.
 * See `conformance/README.md` for the case format and the canonical JSON rules.
 */
class ConformanceTest {
    private val cases = File(System.getProperty("msgkit.conformance"))

    private fun canonicalString(
        value: String,
        out: StringBuilder,
    ) {
        out.append('"')
        for (char in value) {
            when {
                char == '"' -> out.append("\\\"")
                char == '\' -> out.append("\\\\")
                char == '\b' -> out.append("\b")
                char == '\u000c' -> out.append("\f")
                char == '\n' -> out.append("\n")
                char == '\r' -> out.append("\r")
                char == '\t' -> out.append("\t")
                char.code < CONTROL_LIMIT -> out.append("\u%04x".format(char.code))
                else -> out.append(char)
            }
        }
        out.append('"')
    }

    // Order by Unicode code point (String.compareTo compares UTF-16 code units).
    private fun compareCodePoints(
        a: String,
        b: String,
    ): Int {
        val x = a.codePoints().toArray()
        val y = b.codePoints().toArray()
        for (i in 0 until minOf(x.size, y.size)) {
            if (x[i] != y[i]) return x[i] - y[i]
        }
        return x.size - y.size
    }

    private fun canonical(
        value: JsonElement,
        out: StringBuilder,
    ) {
        when (value) {
            is JsonNull -> out.append("null")
            is JsonPrimitive ->
                when {
                    value.isString -> canonicalString(value.content, out)
                    value.content == "true" || value.content == "false" -> out.append(value.content)
                    else -> {
                        val integer = value.content.all { it.isDigit() || it == '-' }
                        assertTrue(integer, "no floats in conformance output: $value")
                        out.append(value.content)
                    }
                }
            is JsonArray -> {
                out.append('[')
                value.forEachIndexed { i, item ->
                    if (i > 0) out.append(',')
                    canonical(item, out)
                }
                out.append(']')
            }
            is JsonObject -> {
                out.append('{')
                value.keys.sortedWith(::compareCodePoints).forEachIndexed { i, key ->
                    if (i > 0) out.append(',')
                    canonicalString(key, out)
                    out.append(':')
                    canonical(value.getValue(key), out)
                }
                out.append('}')
            }
        }
    }

    private fun loadInput(
        dir: File,
        spec: JsonObject,
    ): JsonObject {
        val entries = LinkedHashMap<String, JsonElement>(spec.getValue("input").jsonObject)
        spec["files"]?.jsonObject?.forEach { (key, name) ->
            entries[key] = JsonPrimitive(File(dir, (name as JsonPrimitive).content).readText(Charsets.UTF_8))
        }
        spec["json_files"]?.jsonObject?.forEach { (key, name) ->
            entries[key] = Json.parseToJsonElement(File(dir, (name as JsonPrimitive).content).readText(Charsets.UTF_8))
        }
        return JsonObject(entries)
    }

    @Test
    fun `every case is reproduced byte for byte`() {
        val dirs = cases.listFiles { f -> f.isDirectory }!!.sortedBy { it.name }
        assertTrue(dirs.size >= MIN_CASES, "expected the full suite, found ${dirs.size} cases")
        for (dir in dirs) {
            val spec = Json.parseToJsonElement(File(dir, "case.json").readText(Charsets.UTF_8)).jsonObject
            val op = (spec.getValue("op") as JsonPrimitive).content
            val raw = Msgkit.dispatchRaw(op, loadInput(dir, spec).toString())
            val out = StringBuilder()
            canonical(Json.parseToJsonElement(raw), out)
            out.append('\n')
            assertEquals(File(dir, "expected.json").readText(Charsets.UTF_8), out.toString(), "case ${dir.name}")
        }
    }

    private companion object {
        const val CONTROL_LIMIT = 0x20
        const val MIN_CASES = 40
    }
}
