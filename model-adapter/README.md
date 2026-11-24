# model-adapter

The one place a model call is made (decision 0003, TD 3.3 and 9). Built ahead of the
companion service (#29). When `service/` lands, this folder moves to
`service/src/model/` and `createChatHandler` is mounted behind the sign-in check.

- The route, ceiling and models are config (`src/config.ts`). Changing route is a config change.
- Only a stub route exists until #19 names the real one. Nothing real leaves the device.
- Above the ceiling, or an unknown classification, is refused with 403. Never downgraded.
- Logs hold metadata only (`src/log.ts`). Request and reply text never reaches a log.
- `test/no-client-creds.test.js` fails if `shell/` or `executor-macos/` holds a model
  credential or a direct model URL.

Run the tests: `npm test`. There is no server entry yet; the service wires `createChatHandler` into its HTTP stack.
