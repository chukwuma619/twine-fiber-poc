import 'dart:convert';

import 'package:http/http.dart' as http;

import 'models.dart';

class FiberApi {
  FiberApi({http.Client? client}) : _client = client ?? http.Client();

  final http.Client _client;

  Future<String> nodePubkey(String rpcUrl) async {
    final result = await call(rpcUrl, 'node_info');
    final pubkey = result['pubkey'] as String?;
    if (pubkey == null || pubkey.isEmpty) {
      throw const FiberException('node_info missing pubkey');
    }
    return pubkey;
  }

  Future<List<FiberChannel>> listChannels(String rpcUrl) async {
    final result = await call(rpcUrl, 'list_channels', <String, Object?>{});
    final raw = result['channels'];
    if (raw is! List) {
      return const [];
    }
    return [
      for (final item in raw)
        if (item is Map<String, dynamic>)
          FiberChannel(
            peerPubkey: item['pubkey'] as String? ?? '',
            open: _ready(item),
            channelId: item['channel_id'] as String?,
            localBalance: _asString(item['local_balance']),
            remoteBalance: _asString(item['remote_balance']),
          ),
    ];
  }

  Future<FiberChannel?> channelTo(String rpcUrl, String peerPubkey) async {
    final want = _norm(peerPubkey);
    final channels = await listChannels(rpcUrl);
    FiberChannel? fallback;
    for (final channel in channels) {
      if (_norm(channel.peerPubkey) != want) {
        continue;
      }
      if (channel.open) {
        return channel;
      }
      fallback ??= channel;
    }
    return fallback;
  }

  Future<void> connectPeer(
    String rpcUrl, {
    required String pubkey,
    required String address,
  }) async {
    try {
      await call(rpcUrl, 'connect_peer', {
        'pubkey': pubkey,
        'address': address,
        'save': true,
      });
    } on FiberException catch (err) {
      final lower = err.message.toLowerCase();
      if (lower.contains('already') || lower.contains('connected')) {
        return;
      }
      rethrow;
    }
  }

  Future<void> openChannel(
    String rpcUrl, {
    required String pubkey,
    required String fundingHex,
  }) async {
    await call(rpcUrl, 'open_channel', {
      'pubkey': pubkey,
      'funding_amount': fundingHex,
      'public': true,
    });
  }

  Future<void> sendPayment(String rpcUrl, String invoice) async {
    await call(rpcUrl, 'send_payment', {
      'invoice': invoice,
      'max_fee_amount': '0x5f5e100',
    });
  }

  Future<String> newInvoice(
    String rpcUrl, {
    required String amountHex,
    required String description,
  }) async {
    final result = await call(rpcUrl, 'new_invoice', {
      'amount': amountHex,
      'currency': 'Fibt',
      'description': description,
      'hash_algorithm': 'sha256',
    });
    final address = result['invoice_address'] as String?;
    if (address == null || address.isEmpty) {
      throw const FiberException('new_invoice missing invoice_address');
    }
    return address;
  }

  Future<Map<String, dynamic>> call(
    String rpcUrl, [
    String method = '',
    Object? params,
  ]) async {
    final response = await _client.post(
      Uri.parse(rpcUrl.trim()),
      headers: const {'content-type': 'application/json'},
      body: jsonEncode({
        'jsonrpc': '2.0',
        'id': 1,
        'method': method,
        'params': params == null ? <Object?>[] : <Object?>[params],
      }),
    );
    final decoded = response.body.isEmpty ? null : jsonDecode(response.body);
    if (decoded is! Map<String, dynamic>) {
      throw FiberException('fnn $method returned an unexpected body');
    }
    final error = decoded['error'];
    if (error is Map && error['message'] is String) {
      throw FiberException(error['message'] as String);
    }
    final result = decoded['result'];
    if (result is Map<String, dynamic>) {
      return result;
    }
    throw FiberException('fnn $method returned no result');
  }

  bool _ready(Map<String, dynamic> channel) {
    final state = channel['state'];
    if (state is! Map) {
      return false;
    }
    final name = state['state_name'] as String? ?? '';
    return name == 'ChannelReady' || name == 'CHANNEL_READY';
  }

  String? _asString(Object? value) {
    if (value is String) {
      return value;
    }
    if (value is num) {
      return value.toString();
    }
    return null;
  }

  String _norm(String pubkey) {
    var text = pubkey.trim().toLowerCase();
    if (text.startsWith('0x')) {
      text = text.substring(2);
    }
    return text;
  }
}
