import 'dart:convert';

import 'package:http/http.dart' as http;

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

class OrderSnapshot {
  const OrderSnapshot({
    required this.state,
    required this.amount,
    required this.paymentHash,
    required this.invoiceAddress,
    required this.invoiceStatus,
    required this.log,
    required this.chat,
  });

  final String state;
  final String? amount;
  final String? paymentHash;
  final String? invoiceAddress;
  final String? invoiceStatus;
  final List<OrderLogLine> log;
  final List<ChatLine> chat;

  bool get isIdle => state == 'Idle';
  bool get isPending => state == 'Pending';
  bool get isWaitingHold => state == 'WaitingHold';
  bool get isHeld => state == 'Held';
  bool get isWaitingFiat => state == 'WaitingFiat';
  bool get isFiatSent => state == 'FiatSent';
  bool get isReleasing => state == 'Releasing';
  bool get isLeg2Failed => state == 'Leg2Failed';
  bool get isDisputed => state == 'Disputed';
  bool get isSettled => state == 'Settled';
  bool get isExpired => state == 'Expired';
  bool get isOpen =>
      !isIdle &&
      state != 'Cancelled' &&
      state != 'Paid' &&
      state != 'Settled' &&
      state != 'Expired';

  bool get canOpenDispute =>
      isWaitingFiat || isFiatSent || isLeg2Failed;

  bool get sellerWinsLogged =>
      log.any((line) => line.text.contains('solver awarded seller'));

  bool get pathDExpiredLogged =>
      log.any((line) => line.text.contains('path D: hold invoice Expired'));

  /// Open hold that may still transition to Expired via TLC (Path D).
  bool get watchesHoldExpiry {
    switch (state) {
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

  factory OrderSnapshot.fromJson(Map<String, dynamic> json) {
    final rawLog = json['log'];
    final log = <OrderLogLine>[];
    if (rawLog is List) {
      for (final line in rawLog) {
        if (line is Map<String, dynamic>) {
          log.add(OrderLogLine.fromJson(line));
        }
      }
    }
    final rawChat = json['chat'];
    final chat = <ChatLine>[];
    if (rawChat is List) {
      for (final line in rawChat) {
        if (line is Map<String, dynamic>) {
          chat.add(ChatLine.fromJson(line));
        }
      }
    }
    return OrderSnapshot(
      state: json['state'] as String? ?? 'Idle',
      amount: json['amount'] as String?,
      paymentHash: json['payment_hash'] as String?,
      invoiceAddress: json['invoice_address'] as String?,
      invoiceStatus: json['invoice_status'] as String?,
      log: log,
      chat: chat,
    );
  }
}

class DaemonException implements Exception {
  const DaemonException(this.message);

  final String message;

  @override
  String toString() => message;
}

class DaemonApi {
  DaemonApi({http.Client? client}) : _client = client ?? http.Client();

  final http.Client _client;

  Future<OrderSnapshot> fetchOrder(String baseUrl) async {
    final response = await _client.get(Uri.parse('${_root(baseUrl)}/order'));
    return _read(response);
  }

  Future<OrderSnapshot> createOrder(String baseUrl, String amount) async {
    return _post(baseUrl, '/order', body: {'amount': amount});
  }

  Future<OrderSnapshot> demoCancel(String baseUrl) async {
    return _post(baseUrl, '/order/demo_cancel');
  }

  Future<OrderSnapshot> createHold(String baseUrl) async {
    return _post(baseUrl, '/order/hold');
  }

  Future<OrderSnapshot> lock(String baseUrl) async {
    return _post(baseUrl, '/order/lock');
  }

  Future<OrderSnapshot> tryCancel(String baseUrl) async {
    return _post(baseUrl, '/order/try_cancel');
  }

  Future<OrderSnapshot> accept(String baseUrl) async {
    return _post(baseUrl, '/order/accept');
  }

  Future<OrderSnapshot> fiatSent(String baseUrl) async {
    return _post(baseUrl, '/order/fiat_sent');
  }

  Future<OrderSnapshot> release(String baseUrl) async {
    return _post(baseUrl, '/order/release');
  }

  Future<OrderSnapshot> retry(String baseUrl) async {
    return _post(baseUrl, '/order/retry');
  }

  Future<OrderSnapshot> openDispute(String baseUrl) async {
    return _post(baseUrl, '/order/dispute');
  }

  Future<OrderSnapshot> postChat(
    String baseUrl, {
    required String from,
    required String text,
  }) async {
    return _post(baseUrl, '/order/chat', body: {'from': from, 'text': text});
  }

  Future<OrderSnapshot> awardBuyer(String baseUrl) async {
    return _post(baseUrl, '/order/award_buyer');
  }

  Future<OrderSnapshot> awardSeller(String baseUrl) async {
    return _post(baseUrl, '/order/award_seller');
  }

  Future<OrderSnapshot> _post(
    String baseUrl,
    String path, {
    Map<String, Object?>? body,
  }) async {
    final response = await _client.post(
      Uri.parse('${_root(baseUrl)}$path'),
      headers: const {'content-type': 'application/json'},
      body: body == null ? '{}' : jsonEncode(body),
    );
    return _read(response);
  }

  OrderSnapshot _read(http.Response response) {
    final decoded = response.body.isEmpty ? null : jsonDecode(response.body);
    if (response.statusCode >= 400) {
      final message = decoded is Map && decoded['error'] is String
          ? decoded['error'] as String
          : 'daemon returned ${response.statusCode}';
      throw DaemonException(message);
    }
    if (decoded is! Map<String, dynamic>) {
      throw const DaemonException('daemon returned an unexpected order');
    }
    return OrderSnapshot.fromJson(decoded);
  }

  String _root(String baseUrl) {
    final trimmed = baseUrl.trim();
    if (trimmed.endsWith('/')) {
      return trimmed.substring(0, trimmed.length - 1);
    }
    return trimmed;
  }
}
