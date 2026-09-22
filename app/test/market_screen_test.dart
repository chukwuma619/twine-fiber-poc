import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:twine_app/daemon_api.dart';
import 'package:twine_app/fiber_api.dart';
import 'package:twine_app/main.dart';
import 'package:twine_app/settings.dart';
import 'package:twine_app/trade_screen.dart';

const seller = '02aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
const buyer = '02bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';

Map<String, dynamic> adJson() {
  return {
    'id': 'ad1',
    'seller_pubkey': seller,
    'seller_name': 'Ada',
    'available_ckb': '1',
    'fiat': 'NGN',
    'rate': '2000',
    'payment_method': 'Opay',
    'open_trade_id': null,
  };
}

Map<String, dynamic> tradeJson({
  required String state,
  required String sellerPubkey,
  required String buyerPubkey,
}) {
  return {
    'id': 't1',
    'ad_id': 'ad1',
    'seller_pubkey': sellerPubkey,
    'seller_name': 'Ada',
    'buyer_pubkey': buyerPubkey,
    'buyer_name': 'Ben',
    'fiat': 'NGN',
    'rate': '2000',
    'fiat_amount': '2000',
    'amount': '1',
    'payment_method': 'Opay',
    'state': state,
    'payment_hash': '0xhold1',
    'invoice_address': 'fibb1hold',
    'invoice_status': state == 'WaitingHold' ? 'Open' : 'Received',
    'buyer_invoice': null,
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
            name: 'Ben',
            daemonUrl: 'http://127.0.0.1:8080',
            pubkey: buyer,
          ),
        ),
      ),
    );
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));

    expect(find.text('BUY CKB'), findsOneWidget);
    expect(find.text('Order book'), findsOneWidget);
    expect(find.text('SELLING'), findsOneWidget);
    expect(find.text('Ada'), findsOneWidget);
    expect(find.text('1 CKB'), findsOneWidget);
    expect(find.text('2000 NGN / CKB'), findsOneWidget);
    expect(find.textContaining('Opay'), findsOneWidget);
    expect(find.byKey(const Key('take-ad1')), findsOneWidget);

    await tester.tap(find.byKey(const Key('sell-tab')));
    await tester.pump();
    expect(find.byKey(const Key('ad-ad1')), findsNothing);
    expect(find.textContaining('You have no sell offers'), findsOneWidget);
  });

  testWidgets('trade shows Lock only to the seller and Fiat sent only to the buyer', (
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
            sellerPubkey: seller,
            buyerPubkey: buyer,
          ),
        );
      }
      return http.Response(jsonEncode({'error': request.url.path}), 404);
    });

    Future<void> openAs(String pubkey) async {
      final settings = SettingsController(
        persist: false,
        initial: UserSettings(
          name: 'User',
          daemonUrl: 'http://127.0.0.1:8080',
          pubkey: pubkey,
        ),
      );
      await tester.pumpWidget(
        MaterialApp(
          home: TradeScreen(
            key: ValueKey(pubkey),
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

    await openAs(seller);
    expect(find.text('WaitingHold'), findsOneWidget);
    expect(find.byKey(const Key('lock')), findsOneWidget);
    expect(find.byKey(const Key('fiat-sent')), findsNothing);

    state = 'WaitingFiat';
    await openAs(buyer);
    expect(find.text('WaitingFiat'), findsOneWidget);
    expect(find.byKey(const Key('lock')), findsNothing);
    expect(find.byKey(const Key('fiat-sent')), findsOneWidget);
  });
}

DaemonApi widgetDaemon(http.Client client) => DaemonApi(client: client);

FiberApi widgetFiber(http.Client client) => FiberApi(client: client);
