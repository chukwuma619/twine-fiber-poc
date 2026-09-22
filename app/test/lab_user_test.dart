import 'package:flutter_test/flutter_test.dart';
import 'package:twine_app/lab_user.dart';

void main() {
  test('iPhone 17 Pro is lab user A', () {
    expect(
      labUserForDevice(
        const DeviceIdentity(name: 'iPhone 17 Pro', id: 'sim-17'),
      ).seat,
      'A',
    );
    expect(
      labUserForDevice(
        const DeviceIdentity(name: 'iPhone 17 Pro', id: 'sim-17'),
      ).fiberRpc,
      'http://127.0.0.1:8227',
    );
  });

  test('iPhone 18 Pro is lab user B', () {
    expect(
      labUserForDevice(
        const DeviceIdentity(name: 'iPhone 18 Pro', id: 'sim-18'),
      ).seat,
      'B',
    );
    expect(
      labUserForDevice(
        const DeviceIdentity(name: 'iPhone 18 Pro', id: 'sim-18'),
      ).fiberRpc,
      'http://127.0.0.1:8247',
    );
  });
}
