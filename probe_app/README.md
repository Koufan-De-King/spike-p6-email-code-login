# P6 probe app (iOS, Android, macOS)

> **Unverified:** written without a Flutter SDK on the authoring machine. Expect a first-run fix or
> two (the `flutter_web_auth_2` option names in particular). Windows evidence comes from the Rust
> client instead (`cargo run -p another-login`); see the root README.

## First run

```bash
cd probe_app
flutter create --platforms=ios,android,macos --org digital.camer.another --project-name p6_probe .
flutter pub get
```

Then apply the snippets below and run with `flutter run -d <device>`.

## Android (8 / 9)

`android/app/src/main/AndroidManifest.xml`, inside `<application>`:

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

Also set `applicationId "digital.camer.another.p6probe"` and keep `minSdk` ≤ 26.

`<queries>` (Android 11+ package visibility; harmless on 8/9, needed when the same build runs on newer phones):

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
