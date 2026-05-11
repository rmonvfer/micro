# Remote control

An active terminal session can be viewed and controlled from a paired phone. The terminal remains usable while the phone is connected.

## Pair a phone

Pairing is done once per micro data directory:

```text
/remote pair
```

This prints a link for the phone app. To display a QR code instead:

```text
/remote pair qr
```

The pairing secret is stored in `remote-control.json` under micro's data directory with user-only permissions.

## Publish a session

After pairing, run:

```text
/remote
```

The session appears in the phone app. The phone can:

- read the conversation as it streams;
- submit prompts and slash commands;
- steer or stop the current turn;
- queue a follow-up;
- change the model and reasoning level.

Phone input enters the same session and is shown in the terminal transcript.

Running `/remote` again does not open a second connection for the same session.

## Encryption and relay

The phone and machine derive session keys from their shared pairing secret. Messages sent through the relay are encrypted before they leave the endpoint. The relay routes ciphertext and does not receive the session keys.

Use another relay by setting:

```bash
export MICRO_REMOTE_RELAY_URL=https://relay.example.com
```

This changes the relay used for pairing and session traffic. It does not move the local session log.

## Troubleshooting

If `/remote` says no phone is paired, run `/remote pair` again. `MICRO_DIR` changes which pairing file micro reads, so separate profiles require separate pairing.

If a session cannot be published, the error is shown in the terminal. The local session remains active and continues to be recorded.
