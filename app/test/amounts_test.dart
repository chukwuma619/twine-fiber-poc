import 'package:flutter_test/flutter_test.dart';
import 'package:twine_app/amounts.dart';

void main() {
  test('pay over price is CKB with 8-decimal shannon math', () {
    expect(ckbFromPay('2000', '2000'), '1');
    expect(ckbFromPay('1000', '2000'), '0.5');
    expect(payFromCkb('1', '2000'), '2000');
    expect(payFromCkb('0.5', '2000'), '1000');
    expect(takeCap('3000', '1', '2000'), '2000');
    expect(takeCap('1000', '2', '2000'), '1000');
    expect(shannonHex('1'), '0x5f5e100');
    expect(ckbFromShannonHex('0x5f5e100'), '1');
    expect(ckbFromShannonHex('0x2faf080'), '0.5');
    expect(ckbFromShannonHex(''), '0');
  });
}
