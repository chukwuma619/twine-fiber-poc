const int shannonsPerCkb = 100000000;

BigInt shannonsFromDecimal(String raw) {
  final text = raw.trim();
  if (text.isEmpty) {
    throw const FormatException('amount is required');
  }
  final parts = text.split('.');
  if (parts.length > 2) {
    throw const FormatException('amount must be a positive number');
  }
  final whole = parts[0];
  final frac = parts.length == 2 ? parts[1] : '';
  if (whole.isEmpty ||
      !RegExp(r'^\d+$').hasMatch(whole) ||
      (frac.isNotEmpty && !RegExp(r'^\d+$').hasMatch(frac))) {
    throw const FormatException('amount must be a positive number');
  }
  if (frac.length > 8) {
    throw const FormatException('amount supports at most 8 decimal places');
  }
  final padded = frac.padRight(8, '0');
  return BigInt.parse(whole) * BigInt.from(shannonsPerCkb) +
      BigInt.parse(padded.isEmpty ? '0' : padded);
}

String formatDecimal8(BigInt scaled) {
  final whole = scaled ~/ BigInt.from(shannonsPerCkb);
  final frac = scaled % BigInt.from(shannonsPerCkb);
  if (frac == BigInt.zero) {
    return whole.toString();
  }
  final digits = frac.toString().padLeft(8, '0');
  return '$whole.${digits.replaceFirst(RegExp(r'0+$'), '')}';
}

String ckbFromFiat(String fiat, String rate) {
  final fiatValue = shannonsFromDecimal(fiat);
  final rateValue = shannonsFromDecimal(rate);
  if (rateValue == BigInt.zero) {
    throw const FormatException('rate must be greater than zero');
  }
  final ckb = (fiatValue * BigInt.from(shannonsPerCkb)) ~/ rateValue;
  if (ckb == BigInt.zero) {
    throw const FormatException('fiat amount is too small for this rate');
  }
  return formatDecimal8(ckb);
}

String shannonHex(String ckb) {
  return '0x${shannonsFromDecimal(ckb).toRadixString(16)}';
}
