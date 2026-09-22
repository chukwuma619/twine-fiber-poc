import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:twine_app/daemon_api.dart';
import 'package:twine_app/fiber_api.dart';
import 'package:twine_app/main.dart';
import 'package:twine_app/models.dart';
import 'package:twine_app/settings.dart';
import 'package:twine_app/trade_screen.dart';

const tinyPng = <int>[
  0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D,
  0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01,
  0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00,
  0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
  0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49,
  0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

const lister = '02aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
const taker = '02bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';

Map<String, dynamic> adJson() {
  return {
    'id': 'ad1',
    'pubkey': lister,
    'available': '1',
    'currency': 'NGN',
    'price': '2000',
    'min': '1000',
    'max': '2000',
    'payment_method': 'Opay',
    'open_trade_id': null,
  };
}

Map<String, dynamic> tradeJson({
  required String state,
  required String pubkey,
  required String taker,
  Map<String, dynamic>? proof,
  String? disputeFrom,
  String? disputeReason,
  String? buyerInvoice,
}) {
  return {
    'id': 't1',
    'ad_id': 'ad1',
    'pubkey': pubkey,
    'taker': taker,
    'currency': 'NGN',
    'price': '2000',
    'pay_amount': '2000',
    'amount': '1',
    'payment_method': 'Opay',
    'state': state,
    'payment_hash': '0xhold1',
    'invoice_address': 'fibb1hold',
    'invoice_status': state == 'WaitingHold' ? 'Open' : 'Received',
    'buyer_invoice': buyerInvoice,
    'proof': proof,
    'dispute_from': disputeFrom,
    'dispute_reason': disputeReason,
    'accept_by': '2999-01-01T00:00:00Z',
    'pay_by': '2999-01-01T00:00:00Z',
    'log': [
      {'at': 't', 'text': 'hold invoice created H=0xhold1 S sealed in daemon'},
    ],
    'chat': <Map<String, String>>[],
  };
}

http.Response jsonOk(Object body) {
  return http.Response(jsonEncode(body), 200);
}

void main() {
  testWidgets('market renders an open sell ad', (tester) async {
    await tester.binding.setSurfaceSize(const Size(800, 1200));
    addTearDown(() => tester.binding.setSurfaceSize(null));

    final client = MockClient((request) async {
      if (request.url.path == '/ads') {
        return jsonOk([adJson()]);
      }
      if (request.url.path == '/trades') {
        return jsonOk(<Object>[]);
      }
      return http.Response(jsonEncode({'error': request.url.path}), 404);
    });

    await tester.pumpWidget(
      TwineApp(
        client: client,
        persistSettings: false,
        settings: SettingsController(
          persist: false,
          initial: const UserSettings(
            daemonUrl: 'http://127.0.0.1:8080',
            pubkey: taker,
          ),
        ),
      ),
    );
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));

    expect(find.text('BUY CKB'), findsOneWidget);
    expect(find.text('Order book'), findsOneWidget);
    expect(find.text('SELL CKB'), findsOneWidget);
    expect(find.text('2000'), findsOneWidget);
    expect(find.text('NGN/CKB'), findsOneWidget);
    expect(find.text('Available'), findsOneWidget);
    expect(find.text('1 CKB'), findsOneWidget);
    expect(find.text('Limit'), findsOneWidget);
    expect(find.text('1000–2000 NGN'), findsOneWidget);
    expect(find.textContaining('Opay'), findsOneWidget);
    expect(find.byKey(const Key('take-ad1')), findsOneWidget);

    await tester.tap(find.byKey(const Key('sell-tab')));
    await tester.pump();
    expect(find.byKey(const Key('ad-ad1')), findsNothing);
    expect(find.textContaining('You have no sell offers'), findsOneWidget);
  });

  testWidgets('trade shows Lock only to the lister and Fiat sent only to the taker', (
    tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(800, 1600));
    addTearDown(() => tester.binding.setSurfaceSize(null));

    var state = 'WaitingHold';
    final client = MockClient((request) async {
      if (request.url.path == '/trades/t1') {
        return jsonOk(
          tradeJson(
            state: state,
            pubkey: lister,
            taker: taker,
          ),
        );
      }
      return http.Response(jsonEncode({'error': request.url.path}), 404);
    });

    Future<void> openAs(String pubkey) async {
      final settings = SettingsController(
        persist: false,
        initial: UserSettings(
          daemonUrl: 'http://127.0.0.1:8080',
          pubkey: pubkey,
        ),
      );
      await tester.pumpWidget(
        MaterialApp(
          home: TradeScreen(
            key: ValueKey('$pubkey-$state'),
            settings: settings,
            daemon: widgetDaemon(client),
            fiber: widgetFiber(client),
            tradeId: 't1',
          ),
        ),
      );
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 50));
    }

    await openAs(lister);
    expect(find.text('WaitingHold'), findsOneWidget);
    expect(find.byKey(const Key('lock')), findsOneWidget);
    expect(find.text('Accept order'), findsOneWidget);
    expect(find.byKey(const Key('reject-order')), findsOneWidget);
    expect(find.byKey(const Key('fiat-sent')), findsNothing);

    await openAs(taker);
    expect(find.byKey(const Key('lock')), findsNothing);
    expect(find.byKey(const Key('cancel-order')), findsOneWidget);
    expect(find.textContaining('Waiting for the seller'), findsOneWidget);

    state = 'WaitingFiat';
    await openAs(taker);
    expect(find.text('WaitingFiat'), findsOneWidget);
    expect(find.byKey(const Key('lock')), findsNothing);
    expect(find.byKey(const Key('fiat-sent')), findsOneWidget);
    expect(find.text('I have paid'), findsOneWidget);
    expect(
      tester.widget<FilledButton>(find.byKey(const Key('fiat-sent'))).onPressed,
      isNull,
    );
  });

  testWidgets('taker uploads proof then notifies seller', (tester) async {
    await tester.binding.setSurfaceSize(const Size(800, 1600));
    addTearDown(() => tester.binding.setSurfaceSize(null));

    var state = 'WaitingFiat';
    Map<String, dynamic>? fiatBody;
    final client = MockClient((request) async {
      final decoded = request.body.isEmpty ? null : jsonDecode(request.body);
      if (decoded is Map && decoded['method'] == 'new_invoice') {
        return jsonOk({
          'jsonrpc': '2.0',
          'id': 1,
          'result': {'invoice_address': 'fibb1buyer'},
        });
      }
      if (request.url.path == '/trades/t1/fiat_sent') {
        fiatBody = decoded as Map<String, dynamic>?;
        state = 'FiatSent';
        return jsonOk(
          tradeJson(
            state: state,
            pubkey: lister,
            taker: taker,
            proof: {'content_type': 'image/png', 'bytes': tinyPng.length},
            buyerInvoice: 'fibb1buyer',
          ),
        );
      }
      if (request.url.path == '/trades/t1/proof') {
        return http.Response.bytes(tinyPng, 200, headers: {
          'content-type': 'image/png',
        });
      }
      if (request.url.path == '/trades/t1') {
        return jsonOk(
          tradeJson(
            state: state,
            pubkey: lister,
            taker: taker,
            proof: state == 'FiatSent'
                ? {'content_type': 'image/png', 'bytes': tinyPng.length}
                : null,
            buyerInvoice: state == 'FiatSent' ? 'fibb1buyer' : null,
          ),
        );
      }
      return http.Response(jsonEncode({'error': request.url.path}), 404);
    });

    await tester.pumpWidget(
      MaterialApp(
        home: TradeScreen(
          settings: SettingsController(
            persist: false,
            initial: const UserSettings(
              daemonUrl: 'http://127.0.0.1:8080',
              pubkey: taker,
            ),
          ),
          daemon: widgetDaemon(client),
          fiber: widgetFiber(client),
          tradeId: 't1',
          pickProof: () async => const PickedProof(
            bytes: tinyPng,
            contentType: 'image/png',
          ),
        ),
      ),
    );
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));

    expect(
      tester.widget<FilledButton>(find.byKey(const Key('fiat-sent'))).onPressed,
      isNull,
    );
    await tester.tap(find.byKey(const Key('pick-proof')));
    await tester.pump();
    expect(find.text('Receipt selected'), findsOneWidget);
    await tester.tap(find.byKey(const Key('fiat-sent')));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));

    expect(fiatBody?['invoice'], 'fibb1buyer');
    expect(fiatBody?['content_type'], 'image/png');
    expect(fiatBody?['proof_b64'], isNotEmpty);
    expect(find.text('FiatSent'), findsOneWidget);
    expect(find.byKey(const Key('proof-image')), findsOneWidget);
  });

  testWidgets('lister sees payment received after proof and no awards', (
    tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(800, 1600));
    addTearDown(() => tester.binding.setSurfaceSize(null));

    final client = MockClient((request) async {
      if (request.url.path == '/trades/t1/proof') {
        return http.Response.bytes(tinyPng, 200, headers: {
          'content-type': 'image/png',
        });
      }
      if (request.url.path == '/trades/t1') {
        return jsonOk(
          tradeJson(
            state: 'FiatSent',
            pubkey: lister,
            taker: taker,
            proof: {'content_type': 'image/png', 'bytes': tinyPng.length},
            buyerInvoice: 'fibb1buyer',
          ),
        );
      }
      return http.Response(jsonEncode({'error': request.url.path}), 404);
    });

    await tester.pumpWidget(
      MaterialApp(
        home: TradeScreen(
          settings: SettingsController(
            persist: false,
            initial: const UserSettings(
              daemonUrl: 'http://127.0.0.1:8080',
              pubkey: lister,
              operatorTools: true,
            ),
          ),
          daemon: widgetDaemon(client),
          fiber: widgetFiber(client),
          tradeId: 't1',
        ),
      ),
    );
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));

    expect(find.text('Release CKB'), findsOneWidget);
    expect(find.byKey(const Key('release')), findsOneWidget);
    expect(find.byKey(const Key('fiat-sent')), findsNothing);
    expect(find.byKey(const Key('award-buyer')), findsNothing);
    expect(find.byKey(const Key('award-seller')), findsNothing);
    expect(find.byKey(const Key('proof-image')), findsOneWidget);
    expect(find.byKey(const Key('open-dispute')), findsOneWidget);
  });

  testWidgets('award buttons appear only after a filed dispute', (tester) async {
    await tester.binding.setSurfaceSize(const Size(800, 1600));
    addTearDown(() => tester.binding.setSurfaceSize(null));

    final client = MockClient((request) async {
      if (request.url.path == '/trades/t1') {
        return jsonOk(
          tradeJson(
            state: 'Disputed',
            pubkey: lister,
            taker: taker,
            disputeFrom: 'taker',
            disputeReason: 'seller has not released',
          ),
        );
      }
      return http.Response(jsonEncode({'error': request.url.path}), 404);
    });

    await tester.pumpWidget(
      MaterialApp(
        home: TradeScreen(
          settings: SettingsController(
            persist: false,
            initial: const UserSettings(
              daemonUrl: 'http://127.0.0.1:8080',
              pubkey: lister,
              operatorTools: true,
            ),
          ),
          daemon: widgetDaemon(client),
          fiber: widgetFiber(client),
          tradeId: 't1',
        ),
      ),
    );
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));

    expect(find.textContaining('seller has not released'), findsOneWidget);
    expect(find.byKey(const Key('award-buyer')), findsOneWidget);
    expect(find.byKey(const Key('award-seller')), findsOneWidget);
    expect(find.byKey(const Key('release')), findsNothing);
  });

  testWidgets('seller sees a new order when the ad is hidden', (tester) async {
    await tester.binding.setSurfaceSize(const Size(800, 1200));
    addTearDown(() => tester.binding.setSurfaceSize(null));

    final client = MockClient((request) async {
      if (request.url.path == '/ads') {
        return jsonOk(<Object>[]);
      }
      if (request.url.path == '/trades') {
        return jsonOk([
          tradeJson(state: 'WaitingHold', pubkey: lister, taker: taker),
        ]);
      }
      if (request.url.path == '/trades/t1') {
        return jsonOk(
          tradeJson(state: 'WaitingHold', pubkey: lister, taker: taker),
        );
      }
      return http.Response(jsonEncode({'error': request.url.path}), 404);
    });

    await tester.pumpWidget(
      TwineApp(
        client: client,
        persistSettings: false,
        settings: SettingsController(
          persist: false,
          initial: const UserSettings(
            daemonUrl: 'http://127.0.0.1:8080',
            pubkey: lister,
          ),
        ),
      ),
    );
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));

    expect(find.byKey(const Key('new-order-banner')), findsOneWidget);
    expect(find.text('New order — accept'), findsOneWidget);
    expect(find.byKey(const Key('my-trades-badge')), findsOneWidget);

    await tester.tap(find.byKey(const Key('sell-tab')));
    await tester.pump();
    expect(find.textContaining('A buyer already placed an order'), findsOneWidget);
    expect(find.byKey(const Key('open-hidden-trade')), findsOneWidget);
    expect(find.text('Accept order'), findsWidgets);
  });
}

DaemonApi widgetDaemon(http.Client client) => DaemonApi(client: client);

FiberApi widgetFiber(http.Client client) => FiberApi(client: client);
