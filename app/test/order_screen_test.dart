import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:twine_app/main.dart';

Map<String, Object?> idleOrder() => {
      'state': 'Idle',
      'amount': null,
      'payment_hash': null,
      'invoice_address': null,
      'invoice_status': null,
      'log': <Object>[],
      'chat': <Object>[],
    };

void main() {
  testWidgets('create order then a new screen still shows it', (tester) async {
    var pending = false;
    final client = MockClient((request) async {
      if (request.method == 'POST' && request.url.path.endsWith('/order')) {
        pending = true;
      }
      return http.Response(
        jsonEncode(
          pending
              ? {
                  'state': 'Pending',
                  'amount': '1',
                  'payment_hash': null,
                  'invoice_address': null,
                  'invoice_status': null,
                  'log': [
                    {'at': 't', 'text': 'created order for 1 CKB'},
                  ],
                  'chat': <Object>[],
                }
              : idleOrder(),
        ),
        200,
      );
    });

    await tester.pumpWidget(
      TwineApp(client: client, initialUrl: 'http://127.0.0.1:8080'),
    );
    await tester.pumpAndSettle();
    expect(find.text('Idle'), findsOneWidget);

    await tester.tap(find.text('Buyer'));
    await tester.pump();
    expect(find.text('Playing as Buyer'), findsOneWidget);

    await tester.enterText(find.byKey(const Key('amount')), '1');
    await tester.pump();
    await tester.tap(find.byKey(const Key('create-order')));
    await tester.pumpAndSettle();
    expect(find.text('Pending'), findsOneWidget);
    expect(find.textContaining('created order for 1 CKB'), findsOneWidget);
    expect(find.byKey(const Key('create-hold')), findsOneWidget);

    await tester.pumpWidget(
      TwineApp(client: client, initialUrl: 'http://127.0.0.1:8080'),
    );
    await tester.pumpAndSettle();
    expect(find.text('Pending'), findsOneWidget);
    expect(find.text('Amount: 1 CKB'), findsOneWidget);
    expect(find.textContaining('created order for 1 CKB'), findsOneWidget);
    final button = tester.widget<FilledButton>(
      find.byKey(const Key('create-order')),
    );
    expect(button.onPressed, isNull);
  });

  testWidgets('Leg2Failed shows submit-new-invoice message and retry', (
    tester,
  ) async {
    final client = MockClient((request) async {
      if (request.method == 'POST' && request.url.path.endsWith('/order/retry')) {
        return http.Response(
          jsonEncode({
            'state': 'Settled',
            'amount': '1',
            'payment_hash': '0xabc',
            'invoice_address': 'inv',
            'invoice_status': 'Paid',
            'log': [
              {'at': 't', 'text': 'path A complete: state Settled'},
            ],
            'chat': <Object>[],
          }),
          200,
        );
      }
      return http.Response(
        jsonEncode({
          'state': 'Leg2Failed',
          'amount': '1',
          'payment_hash': '0xabc',
          'invoice_address': 'inv',
          'invoice_status': 'Received',
          'log': [
            {
              'at': 't',
              'text':
                  'pay failed: send_payment failed; settle_invoice not called (hold stays Received)',
            },
            {
              'at': 't',
              'text':
                  'path B: buyer should submit a new invoice to retry (POST /order/retry)',
            },
            {
              'at': 't',
              'text':
                  'if the buyer never returns, the seller is refunded when the TLC expires (do not cancel_invoice on Received)',
            },
          ],
          'chat': <Object>[],
        }),
        200,
      );
    });

    await tester.pumpWidget(
      TwineApp(client: client, initialUrl: 'http://127.0.0.1:8080'),
    );
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));

    expect(find.text('Leg2Failed'), findsOneWidget);
    expect(find.byKey(const Key('leg2-failed-message')), findsOneWidget);
    expect(find.textContaining('Submit a new invoice'), findsOneWidget);
    expect(find.byKey(const Key('retry')), findsOneWidget);
    expect(find.byKey(const Key('open-dispute')), findsOneWidget);

    await tester.tap(find.byKey(const Key('retry')));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));
    expect(find.text('Settled'), findsOneWidget);
    expect(find.text('Invoice: Paid'), findsOneWidget);
  });

  testWidgets('solver sees chat and can award; seller-wins shows TLC line', (
    tester,
  ) async {
    var awardedSeller = false;
    final client = MockClient((request) async {
      if (request.method == 'POST' &&
          request.url.path.endsWith('/order/award_seller')) {
        awardedSeller = true;
      }
      if (request.method == 'POST' &&
          request.url.path.endsWith('/order/award_buyer')) {
        return http.Response(
          jsonEncode({
            'error':
                'award refused: hold invoice is Expired; neither buyer nor seller award can run',
          }),
          400,
        );
      }
      return http.Response(
        jsonEncode({
          'state': 'Disputed',
          'amount': '1',
          'payment_hash': '0xhold',
          'invoice_address': 'inv',
          'invoice_status': 'Received',
          'log': [
            {'at': 't', 'text': 'dispute opened (invoice still Received)'},
            if (awardedSeller)
              {
                'at': 't',
                'text':
                    'solver awarded seller: hold stays Received; settle_invoice not called; cancel_invoice not called; seller refund at TLC expiry',
              },
          ],
          'chat': [
            {'at': 't', 'from': 'buyer', 'text': 'I paid fiat'},
            {'at': 't', 'from': 'seller', 'text': 'no you did not'},
          ],
        }),
        200,
      );
    });

    await tester.pumpWidget(
      TwineApp(client: client, initialUrl: 'http://127.0.0.1:8080'),
    );
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));

    await tester.tap(find.text('Solver'));
    await tester.pump();
    expect(find.text('Playing as Solver'), findsOneWidget);
    expect(find.byKey(const Key('dispute-chat-heading')), findsOneWidget);
    expect(find.textContaining('buyer: I paid fiat'), findsOneWidget);
    expect(find.textContaining('seller: no you did not'), findsOneWidget);
    expect(find.byKey(const Key('award-buyer')), findsOneWidget);
    expect(find.byKey(const Key('award-seller')), findsOneWidget);

    await tester.ensureVisible(find.byKey(const Key('award-buyer')));
    await tester.pump();
    await tester.tap(find.byKey(const Key('award-buyer')));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));
    expect(find.textContaining('Expired'), findsOneWidget);

    await tester.ensureVisible(find.byKey(const Key('award-seller')));
    await tester.pump();
    await tester.tap(find.byKey(const Key('award-seller')));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));
    expect(find.byKey(const Key('seller-wins-message')), findsOneWidget);
    expect(find.textContaining('TLC expires'), findsOneWidget);
    expect(find.text('Invoice: Received'), findsOneWidget);
  });

  testWidgets('buyer and seller can open dispute and post chat', (tester) async {
    var disputed = false;
    final chat = <Map<String, String>>[];
    final client = MockClient((request) async {
      if (request.method == 'POST' &&
          request.url.path.endsWith('/order/dispute')) {
        disputed = true;
      }
      if (request.method == 'POST' && request.url.path.endsWith('/order/chat')) {
        final body = jsonDecode(request.body) as Map<String, dynamic>;
        chat.add({
          'from': body['from'] as String,
          'text': body['text'] as String,
        });
      }
      return http.Response(
        jsonEncode({
          'state': disputed ? 'Disputed' : 'WaitingFiat',
          'amount': '1',
          'payment_hash': '0xhold',
          'invoice_address': 'inv',
          'invoice_status': 'Received',
          'log': [
            {'at': 't', 'text': 'buyer accepted'},
            if (disputed)
              {'at': 't', 'text': 'dispute opened (invoice still Received)'},
          ],
          'chat': [
            for (final line in chat)
              {'at': 't', 'from': line['from'], 'text': line['text']},
          ],
        }),
        200,
      );
    });

    await tester.pumpWidget(
      TwineApp(client: client, initialUrl: 'http://127.0.0.1:8080'),
    );
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));

    await tester.tap(find.text('Buyer'));
    await tester.pump();
    expect(find.byKey(const Key('open-dispute')), findsOneWidget);
    await tester.tap(find.byKey(const Key('open-dispute')));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));
    expect(find.text('Disputed'), findsOneWidget);

    await tester.enterText(find.byKey(const Key('chat-input')), 'paid already');
    await tester.pump();
    await tester.tap(find.byKey(const Key('post-chat')));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));
    expect(find.textContaining('buyer: paid already'), findsOneWidget);

    await tester.tap(find.text('Seller'));
    await tester.pump();
    await tester.enterText(find.byKey(const Key('chat-input')), 'not received');
    await tester.pump();
    await tester.tap(find.byKey(const Key('post-chat')));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));
    expect(find.textContaining('seller: not received'), findsOneWidget);
  });

  testWidgets('poll shows Expired and TLC-expiry line without manual refresh', (
    tester,
  ) async {
    var calls = 0;
    final client = MockClient((request) async {
      calls += 1;
      // Held keeps the state/invoice labels on-screen (Disputed chat UI pushes them offstage).
      if (calls == 1) {
        return http.Response(
          jsonEncode({
            'state': 'Held',
            'amount': '1',
            'payment_hash': '0xhold',
            'invoice_address': 'inv',
            'invoice_status': 'Received',
            'log': [
              {
                'at': 't',
                'text':
                    'seller locked: send_payment submitted, get_invoice=Received',
              },
            ],
            'chat': <Object>[],
          }),
          200,
        );
      }
      return http.Response(
        jsonEncode({
          'state': 'Expired',
          'amount': '1',
          'payment_hash': '0xhold',
          'invoice_address': 'inv',
          'invoice_status': 'Expired',
          'log': [
            {
              'at': 't',
              'text':
                  'path D: hold invoice Expired H=0xhold; seller payment failed back; seller refunded because the TLC expired',
            },
            {
              'at': 't',
              'text':
                  'path D: settle_invoice(H, S) after expiry failed as expected: invoice Expired (not a successful settle)',
            },
          ],
          'chat': <Object>[],
        }),
        200,
      );
    });

    await tester.pumpWidget(
      TwineApp(client: client, initialUrl: 'http://127.0.0.1:8080'),
    );
    // Flush the initial GET without advancing the Path D poll timer.
    await tester.pump();
    await tester.pump(Duration.zero);
    expect(find.text('Held'), findsOneWidget);
    expect(find.text('Invoice: Received'), findsOneWidget);
    expect(calls, 1);

    // Advance past the 5s poll interval and flush the follow-up GET.
    await tester.pump(const Duration(seconds: 5));
    await tester.pump();
    await tester.pump(Duration.zero);

    expect(calls, greaterThanOrEqualTo(2));
    expect(find.text('Expired'), findsOneWidget);
    expect(find.text('Invoice: Expired'), findsOneWidget);
    expect(find.byKey(const Key('path-d-expired-message')), findsOneWidget);
    expect(find.textContaining('TLC expired'), findsOneWidget);
    expect(find.textContaining('Seller payment failed back'), findsOneWidget);
  });
}
