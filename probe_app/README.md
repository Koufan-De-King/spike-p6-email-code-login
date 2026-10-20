# P6 probe app (iOS, Android, macOS)

> **Status:** runs on Android 8 and 9 emulators (Flutter 3.47.7, `flutter_web_auth_2` 5.1.0);
> results in `docs/findings.md`. Not yet run on a phone, iOS or macOS. Drive it on an
> emulator with `scripts/android-run.sh`. Windows evidence comes from the Rust
> client instead (`cargo run -p another-login`); see the root README.

## Run

The platform folders (`android/`, `ios/`, `macos/`) are committed and the Android snippets below are
already applied.

```bash
cd probe_app
flutter pub get
flutter run -d <device>
```

## Android (8 / 9)

Already in `android/app/src/main/AndroidManifest.xml`, for reference:

```xml
<activity
    android:name="com.linusu.flutter_web_auth_2.CallbackActivity"
    android:exported="true">
  <intent-filter android:label="flutter_web_auth_2">
    <action android:name="android.intent.action.VIEW" />
    <category android:name="android.intent.category.DEFAULT" />
    <category android:name="android.intent.category.BROWSABLE" />
    <data android:scheme="digital.camer.another" />
  </intent-filter>
  <!-- Variant C only: needs assetlinks.json served with this build's SHA-256 -->
  <intent-filter android:autoVerify="true">
    <action android:name="android.intent.action.VIEW" />
    <category android:name="android.intent.category.DEFAULT" />
    <category android:name="android.intent.category.BROWSABLE" />
    <data android:scheme="https" android:host="auth.another.camer.digital" android:path="/app/callback" />
  </intent-filter>
</activity>
```

**Do not add `android:taskAffinity=""`** to this activity or to `MainActivity`, despite the
plugin's README and Flutter's template. With it, a successful login stays hidden behind Chrome's
Custom Tab (finding F2 in `docs/findings.md`).

`flutter create` sets `applicationId` to `digital.camer.another.p6_probe`; Flutter 3.47 defaults `minSdk` to 24, below Android 8 (26).

Also already there: `<queries>` (Android 11+ package visibility; harmless on 8/9, needed when the same build runs on newer phones):

```xml
<queries>
  <intent><action android:name="android.support.customtabs.action.CustomTabsService" /></intent>
</queries>
```

## iOS (15 / 16)

Nothing to register for the private-use scheme: `ASWebAuthenticationSession` catches the callback
itself, no `CFBundleURLTypes` entry needed. Set the deployment target to 15.0 in Xcode.
Variant C (claimed https) needs iOS 17.4 — on 15/16 it is expected to fail; record that it does.

## macOS (12)

`macos/Runner/DebugProfile.entitlements` and `Release.entitlements`:

```xml
<key>com.apple.security.network.client</key><true/>
```

Deployment target 10.15 or later (ASWebAuthenticationSession on macOS).

## What to record per device

See `docs/test-protocol.md` in the repo root.
