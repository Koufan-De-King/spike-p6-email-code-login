// P6 probe: one screen, one button per redirect variant, timings on screen for the recording.
//
// Analyzes clean on Flutter 3.47.7 with flutter_web_auth_2 5.1.0; see README.md for device setup.

import 'dart:convert';
import 'dart:io' show InternetAddress, Platform, ServerSocket;
import 'dart:math';

import 'package:crypto/crypto.dart';
import 'package:flutter/material.dart';
import 'package:flutter_web_auth_2/flutter_web_auth_2.dart';
import 'package:http/http.dart' as http;
import 'package:shared_preferences/shared_preferences.dart';

const clientId = 'another-app';
const scheme = 'digital.camer.another';
const schemeRedirect = '$scheme:/oauth2redirect';
const httpsHost = 'auth.another.camer.digital';
const httpsRedirect = 'https://$httpsHost/app/callback';

void main() => runApp(const MaterialApp(home: Probe()));

class Probe extends StatefulWidget {
  const Probe({super.key});
  @override
  State<Probe> createState() => _ProbeState();
}

class _ProbeState extends State<Probe> {
  final issuer = TextEditingController(text: 'https://auth.another.camer.digital');
  final label = TextEditingController(text: '${Platform.operatingSystem}-${Platform.operatingSystemVersion}');
  bool ephemeral = true;
  bool busy = false;
  final log = <String>[];

  @override
  void initState() {
    super.initState();
    _detectLostFlow();
  }

  /// A flow that started but never finished means the app process died while the user was in
  /// the mail app (Android "Don't keep activities", or a low-RAM phone). That is THE blocker to
  /// look for on Android 8/9.
  Future<void> _detectLostFlow() async {
    final p = await SharedPreferences.getInstance();
    final pending = p.getString('pending');
    if (pending != null) {
      _say('⚠ previous flow never completed: $pending — process death while away?');
      await p.remove('pending');
    }
  }

  void _say(String s) => setState(() => log.insert(0, '${DateTime.now().toIso8601String().substring(11, 19)}  $s'));

  String _rand(int n) {
    final r = Random.secure();
    return base64UrlEncode(List.generate(n, (_) => r.nextInt(256))).replaceAll('=', '');
  }

  Future<void> _run({required String redirectUri, String? mode}) async {
    setState(() => busy = true);
    final iss = issuer.text.trim().replaceAll(RegExp(r'/$'), '');
    final verifier = _rand(48);
    final challenge = base64UrlEncode(sha256.convert(ascii.encode(verifier)).bytes).replaceAll('=', '');
    final state = _rand(16);
    final nonce = _rand(16);
    final t0 = DateTime.now();
    final prefs = await SharedPreferences.getInstance();
    await prefs.setString('pending', '${label.text} $redirectUri ${mode ?? ''} @ ${t0.toIso8601String()}');

    try {
      final url = Uri.parse('$iss/authorize').replace(queryParameters: {
        'response_type': 'code',
        'client_id': clientId,
        'redirect_uri': redirectUri,
        'scope': 'openid email offline_access',
        'state': state,
        'nonce': nonce,
        'code_challenge': challenge,
        'code_challenge_method': 'S256',
        'prompt': 'login',
        'spike_platform': label.text,
        if (mode != null) 'spike_redirect_mode': mode,
      });
      _say('→ ${Uri.parse(redirectUri).scheme} ${mode ?? ''} ephemeral=$ephemeral');

      final isHttps = redirectUri.startsWith('https://');
      final isLoopback = redirectUri.startsWith('http://127.0.0.1');
      final result = await FlutterWebAuth2.authenticate(
        url: url.toString(),
        // Desktop (Linux/Windows): the plugin listens on 127.0.0.1 at the port in this URL.
        callbackUrlScheme: isLoopback ? redirectUri : (isHttps ? 'https' : scheme),
        options: FlutterWebAuth2Options(
          preferEphemeral: ephemeral,
          // iOS 17.4+ only; on iOS 15/16 an https callback cannot complete the session.
          httpsHost: isHttps ? httpsHost : null,
          httpsPath: isHttps ? '/app/callback' : null,
        ),
      );
      final browserLeg = DateTime.now().difference(t0);
      final cb = Uri.parse(result);
      final q = cb.queryParameters;
      if (q['error'] != null) throw 'authorization error ${q['error']}: ${q['error_description']}';
      if (q['state'] != state) throw 'state mismatch';
      if (q['iss'] != iss) throw 'iss mismatch (RFC 9207)';

      final res = await http.post(Uri.parse('$iss/oauth2/token'), body: {
        'grant_type': 'authorization_code',
        'code': q['code']!,
        'redirect_uri': redirectUri,
        'client_id': clientId,
        'code_verifier': verifier,
      });
      final tok = jsonDecode(res.body) as Map<String, dynamic>;
      if (tok['error'] != null) throw 'token error ${tok['error']}: ${tok['error_description']}';
      // The probe only reads the claims; signature verification lives in the Rust client/core.
      final claims = jsonDecode(utf8.decode(base64Url.decode(base64Url.normalize((tok['id_token'] as String).split('.')[1]))));
      if (claims['nonce'] != nonce) throw 'nonce mismatch';
      _say('✓ ${claims['email']}  browser ${browserLeg.inMilliseconds / 1000}s, total ${DateTime.now().difference(t0).inMilliseconds / 1000}s');
    } catch (e) {
      _say('✗ $e');
    } finally {
      await prefs.remove('pending');
      setState(() => busy = false);
    }
  }

  /// Desktop: ask the OS for a free port, release it, and hand it to the plugin, which re-binds it.
  /// There is a small window where another local process could take the port; PKCE makes a stolen
  /// code useless, so the cost is a failed login, not a compromised one.
  Future<void> _runLoopback() async {
    final probe = await ServerSocket.bind(InternetAddress.loopbackIPv4, 0);
    final port = probe.port;
    await probe.close();
    await _run(redirectUri: 'http://127.0.0.1:$port/callback');
  }

  @override
  Widget build(BuildContext context) {
    final mobile = Platform.isIOS || Platform.isAndroid || Platform.isMacOS;
    // macOS uses ASWebAuthenticationSession (buttons A–C); the loopback listener is Linux/Windows only.
    final desktop = Platform.isLinux || Platform.isWindows;
    return Scaffold(
      appBar: AppBar(title: const Text('P6 probe')),
      body: ListView(padding: const EdgeInsets.all(16), children: [
        TextField(controller: issuer, decoration: const InputDecoration(labelText: 'Issuer')),
        TextField(controller: label, decoration: const InputDecoration(labelText: 'Platform label (shows on /spike)')),
        SwitchListTile(
          title: const Text('Ephemeral session (iOS/macOS)'),
          subtitle: const Text('No Safari cookies, no "wants to use … to sign in" alert'),
          value: ephemeral,
          onChanged: (v) => setState(() => ephemeral = v),
        ),
        if (desktop)
          FilledButton(
            onPressed: busy ? null : _runLoopback,
            child: const Text('D · loopback 127.0.0.1 (desktop)'),
          ),
        FilledButton(
          onPressed: busy || !mobile ? null : () => _run(redirectUri: schemeRedirect),
          child: const Text('A · private-use scheme, 303'),
        ),
        OutlinedButton(
          onPressed: busy || !mobile ? null : () => _run(redirectUri: schemeRedirect, mode: 'interstitial'),
          child: const Text('B · private-use scheme, "Return to app" page'),
        ),
        OutlinedButton(
          onPressed: busy || !mobile ? null : () => _run(redirectUri: httpsRedirect),
          child: const Text('C · claimed https (App/Universal Link)'),
        ),
        const Divider(height: 32),
        for (final l in log) Padding(padding: const EdgeInsets.only(bottom: 6), child: SelectableText(l, style: const TextStyle(fontFamily: 'monospace', fontSize: 12))),
      ]),
    );
  }
}
