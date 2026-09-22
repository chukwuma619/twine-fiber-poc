class OrderLogLine {
  const OrderLogLine({required this.at, required this.text});

  final String at;
  final String text;

  factory OrderLogLine.fromJson(Map<String, dynamic> json) {
    return OrderLogLine(
      at: json['at'] as String? ?? '',
      text: json['text'] as String? ?? '',
    );
  }
}

class ChatLine {
  const ChatLine({required this.at, required this.from, required this.text});

  final String at;
  final String from;
  final String text;

  factory ChatLine.fromJson(Map<String, dynamic> json) {
    return ChatLine(
      at: json['at'] as String? ?? '',
      from: json['from'] as String? ?? '',
      text: json['text'] as String? ?? '',
    );
  }
}

class AdSnapshot {
  const AdSnapshot({
    required this.id,
    required this.pubkey,
    required this.available,
    required this.currency,
    required this.price,
    required this.min,
    required this.max,
    required this.paymentMethod,
    this.openTradeId,
  });

  final String id;
  final String pubkey;
  final String available;
  final String currency;
  final String price;
  final String min;
  final String max;
  final String paymentMethod;
  final String? openTradeId;

  bool isMine(String? other) => samePubkey(pubkey, other);

  factory AdSnapshot.fromJson(Map<String, dynamic> json) {
    return AdSnapshot(
      id: json['id'] as String? ?? '',
      pubkey: _field(json, const ['pubkey', 'seller_pubkey']),
      available: _field(json, const ['available', 'available_ckb']),
      currency: _field(json, const ['currency', 'fiat'], 'NGN'),
      price: _field(json, const ['price', 'rate']),
      min: _field(json, const ['min', 'min_fiat']),
      max: _field(json, const ['max', 'max_fiat']),
      paymentMethod: json['payment_method'] as String? ?? '',
      openTradeId: json['open_trade_id'] as String?,
    );
  }
}

class TradeSnapshot {
  const TradeSnapshot({
    required this.id,
    required this.adId,
    required this.pubkey,
    required this.taker,
    required this.currency,
    required this.price,
    required this.payAmount,
    required this.amount,
    required this.paymentMethod,
    required this.state,
    required this.paymentHash,
    required this.invoiceAddress,
    required this.invoiceStatus,
    required this.buyerInvoice,
    required this.log,
    required this.chat,
  });

  final String id;
  final String adId;
  final String pubkey;
  final String taker;
  final String currency;
  final String price;
  final String payAmount;
  final String amount;
  final String paymentMethod;
  final String state;
  final String? paymentHash;
  final String? invoiceAddress;
  final String? invoiceStatus;
  final String? buyerInvoice;
  final List<OrderLogLine> log;
  final List<ChatLine> chat;

  bool get isWaitingHold => state == 'WaitingHold';
  bool get isWaitingFiat => state == 'WaitingFiat';
  bool get isFiatSent => state == 'FiatSent';
  bool get isReleasing => state == 'Releasing';
  bool get isLeg2Failed => state == 'Leg2Failed';
  bool get isDisputed => state == 'Disputed';
  bool get isSettled => state == 'Settled';
  bool get isExpired => state == 'Expired';

  bool get canOpenDispute => isWaitingFiat || isFiatSent || isLeg2Failed;

  bool get sellerWinsLogged =>
      log.any((line) => line.text.contains('solver awarded seller'));

  bool get pathDExpiredLogged =>
      log.any((line) => line.text.contains('path D: hold invoice Expired'));

  bool get watchesHoldExpiry {
    switch (state) {
      case 'WaitingHold':
      case 'Held':
      case 'WaitingFiat':
      case 'FiatSent':
      case 'Leg2Failed':
      case 'Disputed':
      case 'Releasing':
        return true;
      default:
        return false;
    }
  }

  bool isLister(String? other) => samePubkey(pubkey, other);

  bool isTaker(String? other) => samePubkey(taker, other);

  factory TradeSnapshot.fromJson(Map<String, dynamic> json) {
    return TradeSnapshot(
      id: json['id'] as String? ?? '',
      adId: json['ad_id'] as String? ?? '',
      pubkey: _field(json, const ['pubkey', 'seller_pubkey']),
      taker: _field(json, const ['taker', 'buyer_pubkey']),
      currency: _field(json, const ['currency', 'fiat']),
      price: _field(json, const ['price', 'rate']),
      payAmount: _field(json, const ['pay_amount', 'fiat_amount']),
      amount: json['amount'] as String? ?? '',
      paymentMethod: json['payment_method'] as String? ?? '',
      state: json['state'] as String? ?? 'Idle',
      paymentHash: json['payment_hash'] as String?,
      invoiceAddress: json['invoice_address'] as String?,
      invoiceStatus: json['invoice_status'] as String?,
      buyerInvoice: json['buyer_invoice'] as String?,
      log: _lines(json['log'], OrderLogLine.fromJson),
      chat: _lines(json['chat'], ChatLine.fromJson),
    );
  }
}

class TwineInfo {
  const TwineInfo({
    required this.rpc,
    required this.p2pAddress,
    this.pubkey,
    this.nodeName,
    this.error,
  });

  final String rpc;
  final String p2pAddress;
  final String? pubkey;
  final String? nodeName;
  final String? error;

  factory TwineInfo.fromJson(Map<String, dynamic> json) {
    return TwineInfo(
      rpc: json['rpc'] as String? ?? '',
      p2pAddress: json['p2p_address'] as String? ?? '',
      pubkey: json['pubkey'] as String?,
      nodeName: json['node_name'] as String?,
      error: json['error'] as String?,
    );
  }
}

class ConnectResult {
  const ConnectResult({
    required this.connected,
    required this.channelOpen,
    required this.message,
    this.channelId,
  });

  final bool connected;
  final bool channelOpen;
  final String message;
  final String? channelId;

  factory ConnectResult.fromJson(Map<String, dynamic> json) {
    return ConnectResult(
      connected: json['connected'] as bool? ?? false,
      channelOpen: json['channel_open'] as bool? ?? false,
      message: json['message'] as String? ?? '',
      channelId: json['channel_id'] as String?,
    );
  }
}

class FiberChannel {
  const FiberChannel({
    required this.peerPubkey,
    required this.open,
    this.channelId,
    this.localBalance,
    this.remoteBalance,
  });

  final String peerPubkey;
  final bool open;
  final String? channelId;
  final String? localBalance;
  final String? remoteBalance;
}

class DaemonException implements Exception {
  const DaemonException(this.message);

  final String message;

  @override
  String toString() => message;
}

class FiberException implements Exception {
  const FiberException(this.message);

  final String message;

  @override
  String toString() => message;
}

String _field(Map<String, dynamic> json, List<String> keys, [String fallback = '']) {
  for (final key in keys) {
    final value = json[key];
    if (value is String && value.isNotEmpty) {
      return value;
    }
  }
  return fallback;
}

List<T> _lines<T>(
  Object? raw,
  T Function(Map<String, dynamic> json) parse,
) {
  final items = <T>[];
  if (raw is List) {
    for (final line in raw) {
      if (line is Map<String, dynamic>) {
        items.add(parse(line));
      }
    }
  }
  return items;
}

bool samePubkey(String left, String? right) {
  if (right == null || right.trim().isEmpty) {
    return false;
  }
  return _norm(left) == _norm(right);
}

String _norm(String pubkey) {
  var text = pubkey.trim().toLowerCase();
  if (text.startsWith('0x')) {
    text = text.substring(2);
  }
  return text;
}
