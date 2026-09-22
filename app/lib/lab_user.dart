import 'package:device_info_plus/device_info_plus.dart';
import 'package:flutter/foundation.dart';

class LabUser {
  const LabUser({
    required this.seat,
    required this.fiberRpc,
    required this.p2pAddress,
  });

  final String seat;
  final String fiberRpc;
  final String p2pAddress;

  String get label =>
      seat == 'B' ? 'User B · buyer node' : 'User A · seller node';
}

const labUserA = LabUser(
  seat: 'A',
  fiberRpc: 'http://127.0.0.1:8227',
  p2pAddress: '/ip4/127.0.0.1/tcp/8228',
);

const labUserB = LabUser(
  seat: 'B',
  fiberRpc: 'http://127.0.0.1:8247',
  p2pAddress: '/ip4/127.0.0.1/tcp/8248',
);

class DeviceIdentity {
  const DeviceIdentity({required this.name, required this.id});

  final String name;
  final String id;
}

LabUser labUserForDevice(DeviceIdentity device) {
  final name = device.name.toLowerCase();
  if (name.contains('18') ||
      name.contains('buyer') ||
      name.contains('user b')) {
    return labUserB;
  }
  if (name.contains('17') ||
      name.contains('seller') ||
      name.contains('user a')) {
    return labUserA;
  }
  final sum = device.id.codeUnits.fold<int>(0, (a, b) => a + b);
  return sum.isEven ? labUserA : labUserB;
}

String fiberRpcOnThisPhone(String hostRpc) {
  if (!kIsWeb && defaultTargetPlatform == TargetPlatform.android) {
    return hostRpc.replaceFirst('127.0.0.1', '10.0.2.2');
  }
  return hostRpc;
}

Future<DeviceIdentity> readDeviceIdentity() async {
  final plugin = DeviceInfoPlugin();
  switch (defaultTargetPlatform) {
    case TargetPlatform.iOS:
      final info = await plugin.iosInfo;
      return DeviceIdentity(
        name: info.name,
        id: info.identifierForVendor ?? info.name,
      );
    case TargetPlatform.android:
      final info = await plugin.androidInfo;
      final name = info.name.isNotEmpty ? info.name : info.model;
      return DeviceIdentity(name: name, id: info.id);
    case TargetPlatform.macOS:
    case TargetPlatform.linux:
    case TargetPlatform.windows:
    case TargetPlatform.fuchsia:
      return const DeviceIdentity(name: 'desktop', id: 'desktop');
  }
}
