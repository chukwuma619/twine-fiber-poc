import 'package:flutter/material.dart';
import 'package:http/http.dart' as http;

import 'daemon_api.dart';
import 'fiber_api.dart';
import 'home_shell.dart';
import 'settings.dart';

class TwineApp extends StatefulWidget {
  const TwineApp({
    super.key,
    this.client,
    this.settings,
    this.persistSettings = true,
  });

  final http.Client? client;
  final SettingsController? settings;
  final bool persistSettings;

  @override
  State<TwineApp> createState() => _TwineAppState();
}

class _TwineAppState extends State<TwineApp> {
  late final SettingsController _settings =
      widget.settings ?? SettingsController(persist: widget.persistSettings);
  late final DaemonApi _daemon = DaemonApi(client: widget.client);
  late final FiberApi _fiber = FiberApi(client: widget.client);

  @override
  void initState() {
    super.initState();
    _start();
  }

  Future<void> _start() async {
    await _settings.load();
    if (!mounted || !_settings.persist) {
      return;
    }
    try {
      final pubkey = await _fiber.nodePubkey(_settings.settings.fiberRpc);
      if (!mounted || pubkey == _settings.settings.pubkey) {
        return;
      }
      await _settings.update(_settings.settings.copyWith(pubkey: pubkey));
    } catch (_) {
      // Settings still lets them tap Read this phone if the node is down.
    }
  }

  @override
  void dispose() {
    if (widget.settings == null) {
      _settings.dispose();
    }
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Twine',
      home: HomeShell(
        settings: _settings,
        daemon: _daemon,
        fiber: _fiber,
      ),
    );
  }
}

void main() {
  runApp(const TwineApp());
}
