import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:twine_app/main.dart';

/// In-memory daemon. One order, the same transitions the Rust daemon uses.
class TradeScript {
  String state = 'Idle';
  String? amount;
  String? hash;
  String? invoice;
  String? status;
  final List<Map<String, String>> log = [];
  final List<Map<String, String>> chat = [];
  var orders = 0;
  var failNextRelease = true;
  var expireOnGet = false;

  bool get isOpen =>
      state != 'Idle' &&
      state != 'Cancelled' &&
      state != 'Paid' &&
      state != 'Settled' &&
      state != 'Expired';

  Future<http.Response> handle(http.Request request) async {
    final path = request.url.path;
    if (request.method == 'GET' && path == '/order') {
      _maybeExpire();
      return _ok();
    }
    return switch (path) {
      '/order' => _create(request),
      '/order/demo_cancel' => _demoCancel(),
      '/order/hold' => _hold(),
      '/order/lock' => _lock(),
      '/order/try_cancel' => _tryCancel(),
      '/order/accept' => _accept(),
      '/order/fiat_sent' => _fiatSent(),
      '/order/release' => _release(),
      '/order/retry' => _retry(),
      '/order/dispute' => _dispute(),
      '/order/chat' => _chat(request),
      '/order/award_seller' => _awardSeller(),
      _ => _err(404, 'unexpected ${request.method} $path'),
    };
  }

  http.Response _create(http.Request request) {
    if (isOpen) {
      return _err(409, 'an order is already open');
    }
    final body = jsonDecode(request.body) as Map<String, dynamic>;
    orders += 1;
    state = 'Pending';
    amount = body['amount'] as String?;
    hash = null;
    invoice = null;
    status = null;
    log
      ..clear()
      ..add({'at': 't', 'text': 'created order for $amount CKB'});
    chat.clear();
    return _ok();
  }

  http.Response _demoCancel() {
    _add(
      'demo cancel: unpaid invoice Cancelled (live order unchanged, still Pending)',
    );
    return _ok();
  }

  http.Response _hold() {
    state = 'WaitingHold';
    hash = '0xhold$orders';
    invoice = 'fibb1$hash';
    status = 'Open';
    _add(
      'hold invoice created H=$hash final_expiry_delta=57600000ms (0x36ee800) '
      'S sealed in daemon (never sent to app)',
    );
    return _ok();
  }

  http.Response _lock() {
    state = 'Held';
    status = 'Received';
    _add('seller locked: send_payment submitted, get_invoice=Received');
    _add(
      'H=$hash S still sealed in daemon; Twine spendable balance unchanged until settle',
    );
    return _ok();
  }

  http.Response _tryCancel() {
    _add(
      'cancel_invoice not applied (invoice=Received): only legal while Open; '
      'after Received the seller refund is TLC expiry, not cancel',
    );
    return _ok();
  }

  http.Response _accept() {
    state = 'WaitingFiat';
    _add('buyer accepted; start fiat timer in the app');
    return _ok();
  }

  http.Response _fiatSent() {
    state = 'FiatSent';
    _add('buyer marked fiat sent');
    return _ok();
  }

  http.Response _release() {
    if (failNextRelease) {
      failNextRelease = false;
      state = 'Leg2Failed';
      status = 'Received';
      _add(
        'pay failed: no route to buyer; settle_invoice not called (hold stays Received)',
      );
      _add('if the buyer never returns, the seller is refunded when the TLC expires');
      return _ok();
    }
    return _settled('path A complete: state Settled');
  }

  http.Response _retry() => _settled('path A complete: state Settled');

  http.Response _dispute() {
    state = 'Disputed';
    _add('dispute opened (invoice still Received)');
    return _ok();
  }

  http.Response _chat(http.Request request) {
    final body = jsonDecode(request.body) as Map<String, dynamic>;
    final from = body['from'] as String;
    final text = body['text'] as String;
    chat.add({'at': 't', 'from': from, 'text': text});
    _add('chat ($from): $text');
    return _ok();
  }

  http.Response _awardSeller() {
    if (status == 'Expired') {
      return _err(
        400,
        'award refused: hold invoice is Expired; neither buyer nor seller award can run',
      );
    }
    _add(
      'solver awarded seller: hold stays Received; settle_invoice not called; '
      'cancel_invoice not called; seller refund at TLC expiry',
    );
    return _ok();
  }

  http.Response _settled(String done) {
    state = 'Settled';
    status = 'Paid';
    _add('pay: twine send_payment submitted');
    _add('settle: settle_invoice(H, S) submitted on twine');
    _add(done);
    return _ok();
  }

  void _maybeExpire() {
    if (!expireOnGet || state != 'Disputed') {
      return;
    }
    state = 'Expired';
    status = 'Expired';
    _add(
      'path D: hold invoice Expired H=$hash; seller payment failed back; '
      'seller refunded because the TLC expired',
    );
    _add(
      'path D: settle_invoice(H, S) after expiry failed as expected: invoice Expired '
      '(not a successful settle)',
    );
    _add('path D: cancel_invoice not called');
  }

  void _add(String text) {
    log.add({'at': 't', 'text': text});
  }

  http.Response _ok() {
    return http.Response(
      jsonEncode({
        'state': state,
        'amount': amount,
        'payment_hash': hash,
        'invoice_address': invoice,
        'invoice_status': status,
        'log': log,
        'chat': chat,
      }),
      200,
    );
  }

  http.Response _err(int code, String message) {
    return http.Response(jsonEncode({'error': message}), code);
  }
}

void main() {
  testWidgets('one trade walks hold, release, dispute, and expiry', (
    tester,
  ) async {
    await tester.binding.setSurfaceSize(const Size(800, 1600));
    addTearDown(() => tester.binding.setSurfaceSize(null));

    final script = TradeScript();
    final client = MockClient(script.handle);
    Future<void> launch() async {
      await tester.pumpWidget(
        TwineApp(client: client, initialUrl: 'http://127.0.0.1:8080'),
      );
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 50));
    }

    Future<void> tap(Key key) async {
      final finder = find.byKey(key);
      await tester.ensureVisible(finder);
      await tester.pump();
      await tester.tap(finder);
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 50));
    }

    Future<void> role(String name) async {
      await tester.tap(find.text(name));
      await tester.pump();
    }

    await launch();
    expect(find.text('Idle'), findsOneWidget);
    expect(find.text('Playing as Seller'), findsOneWidget);

    await tester.enterText(find.byKey(const Key('amount')), '1');
    await tester.pump();
    await tap(const Key('create-order'));
    expect(find.text('Pending'), findsOneWidget);
    expect(find.textContaining('created order for 1 CKB'), findsOneWidget);
    expect(
      tester.widget<FilledButton>(find.byKey(const Key('create-order'))).onPressed,
      isNull,
    );

    await tap(const Key('demo-cancel'));
    expect(find.text('Pending'), findsOneWidget);
    expect(find.textContaining('unpaid invoice Cancelled'), findsOneWidget);

    await tap(const Key('create-hold'));
    expect(find.text('WaitingHold'), findsOneWidget);
    expect(find.textContaining('H: 0xhold1'), findsOneWidget);
    expect(find.textContaining('S sealed'), findsOneWidget);
    expect(find.textContaining('payment_preimage'), findsNothing);

    await tap(const Key('lock'));
    expect(find.text('Held'), findsOneWidget);
    expect(find.text('Invoice: Received'), findsOneWidget);

    await tap(const Key('try-cancel'));
    expect(find.text('Held'), findsOneWidget);
    expect(find.textContaining('cancel_invoice not applied'), findsOneWidget);
    expect(find.textContaining('TLC expiry'), findsOneWidget);

    await role('Buyer');
    expect(find.text('Playing as Buyer'), findsOneWidget);
    await tap(const Key('accept'));
    expect(find.text('WaitingFiat'), findsOneWidget);
    expect(find.byKey(const Key('fiat-timer')), findsOneWidget);

    await tap(const Key('fiat-sent'));
    expect(find.text('FiatSent'), findsOneWidget);

    await role('Seller');
    await tap(const Key('release'));
    expect(find.text('Leg2Failed'), findsOneWidget);
    expect(find.byKey(const Key('leg2-failed-message')), findsOneWidget);
    expect(find.textContaining('Submit a new invoice'), findsOneWidget);
    expect(find.byKey(const Key('open-dispute')), findsOneWidget);
    expect(find.textContaining('settle_invoice not called'), findsOneWidget);

    await tap(const Key('retry'));
    expect(find.text('Settled'), findsOneWidget);
    expect(find.text('Invoice: Paid'), findsOneWidget);
    expect(find.textContaining('pay:'), findsWidgets);
    expect(find.textContaining('settle:'), findsWidgets);
    expect(find.textContaining('path A complete'), findsOneWidget);

    await launch();
    expect(find.text('Settled'), findsOneWidget);
    expect(find.text('Invoice: Paid'), findsOneWidget);
    expect(find.textContaining('payment_preimage'), findsNothing);

    await tester.enterText(find.byKey(const Key('amount')), '1');
    await tester.pump();
    await tap(const Key('create-order'));
    await tap(const Key('create-hold'));
    await tap(const Key('lock'));
    expect(find.text('Held'), findsOneWidget);

    await role('Buyer');
    await tap(const Key('accept'));
    await tap(const Key('open-dispute'));
    expect(find.text('Disputed'), findsOneWidget);
    await tester.enterText(find.byKey(const Key('chat-input')), 'I sent the fiat');
    await tester.pump();
    await tap(const Key('post-chat'));
    expect(find.textContaining('buyer: I sent the fiat'), findsOneWidget);

    await role('Seller');
    await tester.enterText(find.byKey(const Key('chat-input')), 'I never got it');
    await tester.pump();
    await tap(const Key('post-chat'));
    expect(find.textContaining('seller: I never got it'), findsOneWidget);

    await role('Solver');
    expect(find.text('Playing as Solver'), findsOneWidget);
    expect(find.textContaining('buyer: I sent the fiat'), findsOneWidget);
    expect(find.textContaining('seller: I never got it'), findsOneWidget);
    await tap(const Key('award-seller'));
    expect(find.byKey(const Key('seller-wins-message')), findsOneWidget);
    expect(find.text('Invoice: Received'), findsOneWidget);
    expect(find.textContaining('TLC expiry'), findsWidgets);

    script.expireOnGet = true;
    for (var second = 0; second < 8; second++) {
      await tester.pump(const Duration(seconds: 1));
      await tester.pump();
    }

    expect(find.text('Expired'), findsOneWidget);
    expect(find.text('Invoice: Expired'), findsOneWidget);
    expect(find.byKey(const Key('path-d-expired-message')), findsOneWidget);
    expect(find.textContaining('Seller payment failed back'), findsOneWidget);
    expect(find.textContaining('cancel_invoice not called'), findsWidgets);
    expect(find.textContaining('payment_preimage'), findsNothing);
  });
}
