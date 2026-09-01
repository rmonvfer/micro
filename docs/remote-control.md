# Remote control

An active terminal session can be viewed and controlled from a paired phone. The terminal remains usable while the phone is connected.

## Pair a phone

Pairing requires the Parley phone app and is done once per micro data directory:

```text
/remote pair
```

This prints a QR code containing a random 32-byte pairing secret. In Parley, open the pairing scanner and scan the code. The QR code is a credential; do not share it or include it in screenshots.

`remote-control.json` is written immediately under micro's data directory with user-only permissions. Running `/remote pair` again replaces the stored pairing, so micro no longer connects with the previous QR code. `/remote pair qr` remains an alias for the same command.

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

The phone and machine derive directional keys from their shared pairing secret. Payloads are encrypted before they leave either endpoint. The relay still sees connection metadata, pairing and session identifiers, timing, and traffic volume.

The pairing secret moves directly from the terminal to the phone in the QR code. The relay receives derived verifiers and cannot substitute a different pairing key. Authenticated nonces remain blocked after reconnect, so a captured frame cannot be accepted twice. A relay can still observe metadata, delay or drop traffic, and deny service.

Use another relay by setting:

```bash
export MICRO_REMOTE_RELAY_URL=https://relay.example.com
```

micro accepts only HTTPS relay URLs and opens channels over WSS. `MICRO_REMOTE_RELAY_URL` selects the relay during pairing, and that URL is stored in `remote-control.json`. Re-pair after changing the variable; active and later sessions use the stored URL. Pairings saved with a plaintext relay are ignored. The local session log is not moved.

## Troubleshooting

If `/remote` says no phone is paired, run `/remote pair` and scan the new QR code. `MICRO_DIR` changes which pairing file micro reads, so separate profiles require separate pairing.

If a session cannot be published, the error is shown in the terminal. The local session remains active and continues to be recorded.
