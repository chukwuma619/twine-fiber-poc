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

class OrderSnapshot {
  const OrderSnapshot({
    required this.state,
    required this.amount,
    required this.paymentHash,
    required this.invoiceAddress,
    required this.invoiceStatus,
    required this.log,
  });

  final String state;
  final String? amount;
  final String? paymentHash;
  final String? invoiceAddress;
  final String? invoiceStatus;
  final List<OrderLogLine> log;

  bool get isIdle => state == 'Idle';
  bool get isPending => state == 'Pending';
  bool get isWaitingHold => state == 'WaitingHold';
  bool get isHeld => state == 'Held';
  bool get isWaitingFiat => state == 'WaitingFiat';
  bool get isFiatSent => state == 'FiatSent';
  bool get isReleasing => state == 'Releasing';
  bool get isSettled => state == 'Settled';
  bool get isOpen =>
      !isIdle &&
      state != 'Cancelled' &&
      state != 'Paid' &&
      state != 'Settled' &&
      state != 'Expired';

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
    return OrderSnapshot(
      state: json['state'] as String? ?? 'Idle',
      amount: json['amount'] as String?,
      paymentHash: json['payment_hash'] as String?,
      invoiceAddress: json['invoice_address'] as String?,
      invoiceStatus: json['invoice_status'] as String?,
      log: log,
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
