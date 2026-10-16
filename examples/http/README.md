# HTTP examples

Start the service:

```bash
MSGKIT_API_TOKEN=dev-token MSGKIT_DB_PATH=./events.db cargo run -p msgkit-service
```

`MSGKIT_DB_PATH` makes webhook events survive restarts (omit it for a throwaway in-memory buffer).

Import a WhatsApp export:

```bash
curl -H "Authorization: Bearer dev-token" --data-binary @chat.txt \
  "http://127.0.0.1:8080/v1/import/whatsapp-export?self_name=Alice&utc_offset_minutes=330"
```

Import a Messenger thread file:

```bash
curl -H "Authorization: Bearer dev-token" -H "Content-Type: application/json" \
  --data-binary @message_1.json \
  "http://127.0.0.1:8080/v1/import/facebook-export?self_name=Alice%20Perera"
```

Simulate a signed WhatsApp webhook (what Meta sends):

```bash
export MSGKIT_WHATSAPP_APP_SECRET=app-secret MSGKIT_WHATSAPP_VERIFY_TOKEN=verify-me
# (restart the service with these set)
BODY=$(cat crates/msgkit-core/tests/fixtures/whatsapp_webhook.json)
SIG="sha256=$(printf '%s' "$BODY" | openssl dgst -sha256 -hmac "$MSGKIT_WHATSAPP_APP_SECRET" | sed 's/^.* //')"
curl -X POST -H "X-Hub-Signature-256: $SIG" -H "Content-Type: application/json" \
  --data-binary "$BODY" http://127.0.0.1:8080/webhooks/whatsapp
curl -H "Authorization: Bearer dev-token" http://127.0.0.1:8080/v1/events
# after your application has stored them, confirm so they are not listed again:
curl -H "Authorization: Bearer dev-token" -H "Content-Type: application/json" \
  -d '{"ids":["<id from the list above>"]}' http://127.0.0.1:8080/v1/events/ack
```
