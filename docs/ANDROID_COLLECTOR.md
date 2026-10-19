# Android collector

msgkit-core cannot read a phone: Android's SMS and call-log providers need Android APIs and runtime permissions. A **collector** is a thin native module in *your* app that reads those rows and hands them to msgkit as a `DeviceBatch`. The core then normalizes, de-duplicates and gives you events.

> The Kotlin below is a reference sketch showing the contract. It is not compiled or tested in this repository (a published AAR collector is on the roadmap).

## Contract

Field names mirror the Android column names ([`schema/options.schema.json`](../schema/options.schema.json) → `device_batch`):

```json
{
  "sms":   [{"id":"11","address":"+94700000000","body":"...","date_ms":1704067200000,"type":1,"thread_id":"5"}],
  "calls": [{"id":"1","number":"+94700000000","name":"Kamal","date_ms":1704067400000,"duration_secs":60,"type":1}]
}
```

* SMS `type`: `1` inbox, `2` sent.
* Call `type`: `1` incoming, `2` outgoing, `3` missed, `4` voicemail, `5` rejected, `6` blocked.
* `date_ms` is milliseconds since the Unix epoch (what Android stores), so no time-zone guessing.

Pass `own_number` and `default_country_code` as options if you want participants in E.164 form.

## Permissions

* `READ_SMS` and `READ_CALL_LOG`, both **runtime** permissions: ask the user and handle denial gracefully.
* Google Play limits these to default SMS/phone apps and a few exceptions. Check the current policy before shipping to Play.

## Sketch (Kotlin)

```kotlin
fun readSms(resolver: ContentResolver, sinceMs: Long): JSONArray {
    val out = JSONArray()
    resolver.query(
        Telephony.Sms.CONTENT_URI,
        arrayOf(Telephony.Sms._ID, Telephony.Sms.ADDRESS, Telephony.Sms.BODY,
                Telephony.Sms.DATE, Telephony.Sms.TYPE, Telephony.Sms.THREAD_ID),
        "${Telephony.Sms.DATE} > ?", arrayOf(sinceMs.toString()),
        "${Telephony.Sms.DATE} ASC"
    )?.use { c ->
        while (c.moveToNext()) out.put(JSONObject()
            .put("id", c.getString(0)).put("address", c.getString(1) ?: "")
            .put("body", c.getString(2) ?: "").put("date_ms", c.getLong(3))
            .put("type", c.getInt(4)).put("thread_id", c.getString(5)))
    }
    return out
}
```

Then call msgkit (through the C ABI via JNI/JNA, or the HTTP sidecar if you process on a server):

```kotlin
val input = JSONObject().put("batch", JSONObject().put("sms", readSms(...)).put("calls", readCalls(...)))
                        .put("options", JSONObject().put("default_country_code", "94"))
val result = nativeCall("normalize_device", input.toString())   // your JNI/JNA wrapper
```

## Incremental sync

Store the highest `date_ms` you have processed and query only newer rows next time. Re-sending old rows is harmless: ids are deterministic, so your store de-duplicates them.
