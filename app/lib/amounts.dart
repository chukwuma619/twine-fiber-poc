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

String ckbFromPay(String pay, String price) {
  final payValue = shannonsFromDecimal(pay);
  final priceValue = shannonsFromDecimal(price);
  if (priceValue == BigInt.zero) {
    throw const FormatException('price must be greater than zero');
  }
  final ckb = (payValue * BigInt.from(shannonsPerCkb)) ~/ priceValue;
  if (ckb == BigInt.zero) {
    throw const FormatException('amount is too small for this price');
  }
  return formatDecimal8(ckb);
}

String payFromCkb(String ckb, String price) {
  final ckbValue = shannonsFromDecimal(ckb);
  final priceValue = shannonsFromDecimal(price);
  if (priceValue == BigInt.zero) {
    throw const FormatException('price must be greater than zero');
  }
  final pay = (ckbValue * priceValue) ~/ BigInt.from(shannonsPerCkb);
  return formatDecimal8(pay);
}

int compareAmount(String left, String right) {
  return shannonsFromDecimal(left).compareTo(shannonsFromDecimal(right));
}

String takeCap(String maxPay, String available, String price) {
  final availablePay = payFromCkb(available, price);
  return compareAmount(maxPay, availablePay) <= 0 ? maxPay : availablePay;
}

String shannonHex(String ckb) {
  return '0x${shannonsFromDecimal(ckb).toRadixString(16)}';
}
