import 'package:flutter/foundation.dart';
import 'package:shared_preferences/shared_preferences.dart';

const String _envFiberRpc = String.fromEnvironment(
  'TWINE_FIBER_RPC',
  defaultValue: 'http://127.0.0.1:8227',
);
const String _envP2p = String.fromEnvironment(
  'TWINE_P2P',
  defaultValue: '/ip4/127.0.0.1/tcp/8228',
);
const String _envPubkey = String.fromEnvironment('TWINE_PUBKEY');
const String _envDaemonUrl = String.fromEnvironment('TWINE_DAEMON_URL');

String defaultDaemonUrl() {
  if (_envDaemonUrl.isNotEmpty) {
    return _envDaemonUrl;
  }
  if (!kIsWeb && defaultTargetPlatform == TargetPlatform.android) {
    return 'http://10.0.2.2:8080';
  }
  return 'http://127.0.0.1:8080';
}

String? defaultPubkey() => _envPubkey.isEmpty ? null : _envPubkey;

class UserSettings {
  const UserSettings({
    this.name = '',
    this.fiberRpc = _envFiberRpc,
    this.daemonUrl = '',
    this.p2pAddress = _envP2p,
    this.operatorTools = false,
    this.pubkey,
  });

  final String name;
  final String fiberRpc;
  final String daemonUrl;
  final String p2pAddress;
  final bool operatorTools;
  final String? pubkey;

  UserSettings copyWith({
    String? name,
    String? fiberRpc,
    String? daemonUrl,
    String? p2pAddress,
    bool? operatorTools,
    String? pubkey,
    bool clearPubkey = false,
  }) {
    return UserSettings(
      name: name ?? this.name,
      fiberRpc: fiberRpc ?? this.fiberRpc,
      daemonUrl: daemonUrl ?? this.daemonUrl,
      p2pAddress: p2pAddress ?? this.p2pAddress,
      operatorTools: operatorTools ?? this.operatorTools,
      pubkey: clearPubkey ? null : (pubkey ?? this.pubkey),
    );
  }
}

class SettingsController extends ChangeNotifier {
  SettingsController({UserSettings? initial, this.persist = true})
    : settings = initial ??
          UserSettings(
            daemonUrl: defaultDaemonUrl(),
            pubkey: defaultPubkey(),
          );

  final bool persist;
  UserSettings settings;
  var loaded = false;

  Future<void> load() async {
    if (!persist) {
      loaded = true;
      notifyListeners();
      return;
    }
    final prefs = await SharedPreferences.getInstance();
    settings = UserSettings(
      name: prefs.getString('name') ?? settings.name,
      fiberRpc: prefs.getString('fiberRpc') ?? settings.fiberRpc,
      daemonUrl: prefs.getString('daemonUrl') ?? settings.daemonUrl,
      p2pAddress: prefs.getString('p2pAddress') ?? settings.p2pAddress,
      operatorTools: prefs.getBool('operatorTools') ?? settings.operatorTools,
      pubkey: prefs.getString('pubkey') ?? settings.pubkey,
    );
    loaded = true;
    notifyListeners();
  }

  Future<void> update(UserSettings next) async {
    settings = next;
    notifyListeners();
    if (!persist) {
      return;
    }
    final prefs = await SharedPreferences.getInstance();
    await prefs.setString('name', next.name);
    await prefs.setString('fiberRpc', next.fiberRpc);
    await prefs.setString('daemonUrl', next.daemonUrl);
    await prefs.setString('p2pAddress', next.p2pAddress);
    await prefs.setBool('operatorTools', next.operatorTools);
    if (next.pubkey == null) {
      await prefs.remove('pubkey');
    } else {
      await prefs.setString('pubkey', next.pubkey!);
    }
  }
}
