import 'package:flutter_test/flutter_test.dart';
import 'package:twine_app/amounts.dart';

void main() {
  test('fiat over rate is CKB with 8-decimal shannon math', () {
    expect(ckbFromFiat('2000', '2000'), '1');
    expect(ckbFromFiat('1000', '2000'), '0.5');
    expect(shannonHex('1'), '0x5f5e100');
  });
}
