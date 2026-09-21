import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:twine_app/main.dart';

void main() {
  testWidgets('create order then a new screen still shows it', (tester) async {
    var pending = false;
    final client = MockClient((request) async {
      if (request.method == 'POST') {
        pending = true;
      }
      return http.Response(
        jsonEncode(
          pending
              ? {
                  'state': 'Pending',
                  'amount': '1',
                  'log': [
                    {'at': 't', 'text': 'created order for 1 CKB'},
                  ],
                }
              : {'state': 'Idle', 'amount': null, 'log': []},
        ),
        200,
      );
    });

    await tester.pumpWidget(TwineApp(client: client));
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

    await tester.pumpWidget(TwineApp(client: client));
    await tester.pumpAndSettle();
    expect(find.text('Pending'), findsOneWidget);
    expect(find.text('Amount: 1 CKB'), findsOneWidget);
    expect(find.textContaining('created order for 1 CKB'), findsOneWidget);
    final button = tester.widget<FilledButton>(
      find.byKey(const Key('create-order')),
    );
    expect(button.onPressed, isNull);
  });
}
