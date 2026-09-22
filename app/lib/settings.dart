import 'package:flutter/foundation.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'lab_user.dart';

const String _envFiberRpc = String.fromEnvironment('TWINE_FIBER_RPC');
const String _envP2p = String.fromEnvironment('TWINE_P2P');
const String _envPubkey = String.fromEnvironment('TWINE_PUBKEY');
const String _envDaemonUrl = String.fromEnvironment('TWINE_DAEMON_URL');
const String _envLabSeat = String.fromEnvironment('TWINE_LAB_SEAT');

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

bool fiberRpcFromDefine() => _envFiberRpc.isNotEmpty;

LabUser labUserFromSeat(String seat) => seat == 'B' ? labUserB : labUserA;

class UserSettings {
  const UserSettings({
    this.fiberRpc = 'http://127.0.0.1:8227',
    this.daemonUrl = '',
    this.p2pAddress = '/ip4/127.0.0.1/tcp/8228',
    this.preferredCurrency = 'NGN',
    this.pubkey,
    this.labSeat,
  });

  final String fiberRpc;
  final String daemonUrl;
  final String p2pAddress;
  final String preferredCurrency;
  final String? pubkey;
  final String? labSeat;

  UserSettings copyWith({
    String? fiberRpc,
    String? daemonUrl,
    String? p2pAddress,
    String? preferredCurrency,
    String? pubkey,
    String? labSeat,
    bool clearPubkey = false,
  }) {
    return UserSettings(
      fiberRpc: fiberRpc ?? this.fiberRpc,
      daemonUrl: daemonUrl ?? this.daemonUrl,
      p2pAddress: p2pAddress ?? this.p2pAddress,
      preferredCurrency: preferredCurrency ?? this.preferredCurrency,
      pubkey: clearPubkey ? null : (pubkey ?? this.pubkey),
      labSeat: labSeat ?? this.labSeat,
    );
  }
}

class SettingsController extends ChangeNotifier {
  SettingsController({
    UserSettings? initial,
    this.persist = true,
    this.readDevice = readDeviceIdentity,
  }) : settings = initial ??
            UserSettings(
              daemonUrl: defaultDaemonUrl(),
              pubkey: defaultPubkey(),
            );

  final bool persist;
  final Future<DeviceIdentity> Function() readDevice;
  UserSettings settings;
  var loaded = false;

  Future<void> load() async {
    if (!persist) {
      loaded = true;
      notifyListeners();
      return;
    }
    final prefs = await SharedPreferences.getInstance();
    final preferredCurrency =
        prefs.getString('preferredCurrency') ?? settings.preferredCurrency;
    final daemonUrl = prefs.getString('daemonUrl') ?? settings.daemonUrl;
    final savedPubkey = prefs.getString('pubkey') ?? settings.pubkey;

    late final UserSettings next;
    if (fiberRpcFromDefine()) {
      final defined = _envLabSeat.isNotEmpty
          ? labUserFromSeat(_envLabSeat.toUpperCase())
          : labUserA;
      next = UserSettings(
        fiberRpc: fiberRpcOnThisPhone(_envFiberRpc),
        daemonUrl: daemonUrl,
        p2pAddress: _envP2p.isNotEmpty ? _envP2p : defined.p2pAddress,
        preferredCurrency: preferredCurrency,
        pubkey: defaultPubkey() ?? savedPubkey,
        labSeat: _envLabSeat.isNotEmpty ? _envLabSeat.toUpperCase() : defined.seat,
      );
    } else {
      final lab = _envLabSeat.isNotEmpty
          ? labUserFromSeat(_envLabSeat.toUpperCase())
          : labUserForDevice(await readDevice());
      next = UserSettings(
        fiberRpc: fiberRpcOnThisPhone(lab.fiberRpc),
        daemonUrl: daemonUrl,
        p2pAddress: lab.p2pAddress,
        preferredCurrency: preferredCurrency,
        pubkey: savedPubkey,
        labSeat: lab.seat,
      );
    }
    loaded = true;
    await update(next);
  }

  Future<void> update(UserSettings next) async {
    settings = next;
    notifyListeners();
    if (!persist) {
      return;
    }
    final prefs = await SharedPreferences.getInstance();
    await prefs.remove('name');
    await prefs.remove('operatorTools');
    await prefs.setString('fiberRpc', next.fiberRpc);
    await prefs.setString('daemonUrl', next.daemonUrl);
    await prefs.setString('p2pAddress', next.p2pAddress);
    await prefs.setString('preferredCurrency', next.preferredCurrency);
    if (next.labSeat == null) {
      await prefs.remove('labSeat');
    } else {
      await prefs.setString('labSeat', next.labSeat!);
    }
    if (next.pubkey == null) {
      await prefs.remove('pubkey');
    } else {
      await prefs.setString('pubkey', next.pubkey!);
    }
  }
}
