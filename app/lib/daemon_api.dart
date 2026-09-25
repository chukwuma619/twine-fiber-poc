import 'dart:convert';

import 'package:http/http.dart' as http;

import 'models.dart';

class DaemonApi {
  DaemonApi({http.Client? client}) : _client = client ?? http.Client();

  final http.Client _client;

  Future<List<AdSnapshot>> listAds(String baseUrl) async {
    final decoded = await _getJson(baseUrl, '/ads');
    if (decoded is! List) {
      throw const DaemonException('daemon returned unexpected ads');
    }
    return [
      for (final item in decoded)
        if (item is Map<String, dynamic>) AdSnapshot.fromJson(item),
    ];
  }

  Future<AdSnapshot> createAd(
    String baseUrl, {
    required String pubkey,
    required String available,
    required String currency,
    required String price,
    required String min,
    required String max,
    required String paymentMethod,
  }) async {
    return AdSnapshot.fromJson(
      await _postMap(baseUrl, '/ads', {
        'pubkey': pubkey,
        'available': available,
        'currency': currency,
        'price': price,
        'min': min,
        'max': max,
        'payment_method': paymentMethod,
      }),
    );
  }

  Future<AdSnapshot> cancelAd(String baseUrl, String id) async {
    return AdSnapshot.fromJson(await _postMap(baseUrl, '/ads/$id/cancel'));
  }

  Future<List<TradeSnapshot>> listTrades(String baseUrl, {String? pubkey}) async {
    final path = pubkey == null || pubkey.isEmpty
        ? '/trades'
        : '/trades?pubkey=${Uri.encodeQueryComponent(pubkey)}';
    final decoded = await _getJson(baseUrl, path);
    if (decoded is! List) {
      throw const DaemonException('daemon returned unexpected trades');
    }
    return [
      for (final item in decoded)
        if (item is Map<String, dynamic>) TradeSnapshot.fromJson(item),
    ];
  }

  Future<TradeSnapshot> fetchTrade(String baseUrl, String id) async {
    return TradeSnapshot.fromJson(
      _asMap(await _getJson(baseUrl, '/trades/$id')),
    );
  }

  Future<TradeSnapshot> createTrade(
    String baseUrl, {
    required String adId,
    required String taker,
    required String payAmount,
  }) async {
    return _trade(baseUrl, '/trades', {
      'ad_id': adId,
      'taker': taker,
      'pay_amount': payAmount,
    });
  }

  Future<TradeSnapshot> markLocked(String baseUrl, String id) {
    return _trade(baseUrl, '/trades/$id/locked');
  }

  Future<TradeSnapshot> cancelTrade(
    String baseUrl,
    String id, {
    required String from,
  }) {
    return _trade(baseUrl, '/trades/$id/cancel', {'from': from});
  }

  Future<TradeSnapshot> fiatSent(
    String baseUrl,
    String id, {
    required String invoice,
    required String proofB64,
    required String contentType,
  }) {
    return _trade(baseUrl, '/trades/$id/fiat_sent', {
      'invoice': invoice,
      'proof_b64': proofB64,
      'content_type': contentType,
    });
  }

  Future<ProofImage> fetchProof(String baseUrl, String id) async {
    final response = await _client.get(Uri.parse('${_root(baseUrl)}/trades/$id/proof'));
    if (response.statusCode >= 400) {
      final decoded = response.body.isEmpty ? null : jsonDecode(response.body);
      final message = decoded is Map && decoded['error'] is String
          ? decoded['error'] as String
          : 'daemon returned ${response.statusCode}';
      throw DaemonException(message);
    }
    return ProofImage(
      bytes: response.bodyBytes,
      contentType: response.headers['content-type'] ?? 'application/octet-stream',
    );
  }

  Future<TradeSnapshot> release(String baseUrl, String id) {
    return _trade(baseUrl, '/trades/$id/release');
  }

  Future<TradeSnapshot> retry(
    String baseUrl,
    String id, {
    required String invoice,
  }) {
    return _trade(baseUrl, '/trades/$id/retry', {'invoice': invoice});
  }

  Future<TradeSnapshot> openDispute(
    String baseUrl,
    String id, {
    required String from,
    required String reason,
  }) {
    return _trade(baseUrl, '/trades/$id/dispute', {
      'from': from,
      'reason': reason,
    });
  }

  Future<TradeSnapshot> postChat(
    String baseUrl,
    String id, {
    required String from,
    required String text,
  }) {
    return _trade(baseUrl, '/trades/$id/chat', {'from': from, 'text': text});
  }

  Future<TradeSnapshot> awardBuyer(
    String baseUrl,
    String id, {
    required String invoice,
  }) {
    return _trade(baseUrl, '/trades/$id/award_buyer', {'invoice': invoice});
  }

  Future<TradeSnapshot> awardSeller(String baseUrl, String id) {
    return _trade(baseUrl, '/trades/$id/award_seller');
  }

  Future<TwineInfo> fetchTwine(String baseUrl) async {
    return TwineInfo.fromJson(_asMap(await _getJson(baseUrl, '/twine')));
  }

  Future<ConnectResult> connect(
    String baseUrl, {
    required String pubkey,
    required String address,
  }) async {
    return ConnectResult.fromJson(
      await _postMap(baseUrl, '/connect', {
        'pubkey': pubkey,
        'address': address,
      }),
    );
  }

  Future<TradeSnapshot> _trade(
    String baseUrl,
    String path, [
    Map<String, Object?>? body,
  ]) async {
    return TradeSnapshot.fromJson(await _postMap(baseUrl, path, body));
  }

  Future<Object?> _getJson(String baseUrl, String path) async {
    final response = await _client.get(Uri.parse('${_root(baseUrl)}$path'));
    return _decode(response);
  }

  Future<Map<String, dynamic>> _postMap(
    String baseUrl,
    String path, [
    Map<String, Object?>? body,
  ]) async {
    final response = await _client.post(
      Uri.parse('${_root(baseUrl)}$path'),
      headers: const {'content-type': 'application/json'},
      body: body == null ? '{}' : jsonEncode(body),
    );
    return _asMap(_decode(response));
  }

  Object? _decode(http.Response response) {
    final decoded = response.body.isEmpty ? null : jsonDecode(response.body);
    if (response.statusCode >= 400) {
      final message = decoded is Map && decoded['error'] is String
          ? decoded['error'] as String
          : 'daemon returned ${response.statusCode}';
      throw DaemonException(message);
    }
    return decoded;
  }

  Map<String, dynamic> _asMap(Object? decoded) {
    if (decoded is! Map<String, dynamic>) {
      throw const DaemonException('daemon returned an unexpected body');
    }
    return decoded;
  }

  String _root(String baseUrl) {
    final trimmed = baseUrl.trim();
    if (trimmed.endsWith('/')) {
      return trimmed.substring(0, trimmed.length - 1);
    }
    return trimmed;
  }
}
