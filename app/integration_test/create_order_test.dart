import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:twine_app/main.dart';

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('create an order, then it is still there after reload', (
    tester,
  ) async {
    await tester.pumpWidget(const TwineApp());
    await tester.pumpAndSettle();

    await tester.tap(find.text('Solver'));
    await tester.pump();
    expect(find.text('Playing as Solver'), findsOneWidget);

    if (find.text('Idle').evaluate().isNotEmpty) {
      await tester.enterText(find.byKey(const Key('amount')), '1');
      await tester.pump();
      await tester.tap(find.byKey(const Key('create-order')));
      await tester.pumpAndSettle();
    }

    expect(find.text('Pending'), findsOneWidget);
    expect(find.textContaining('created order for 1 CKB'), findsOneWidget);

    await tester.pumpWidget(const SizedBox.shrink());
    await tester.pumpWidget(const TwineApp());
    await tester.pumpAndSettle();
    expect(find.text('Pending'), findsOneWidget);
    expect(find.text('Amount: 1 CKB'), findsOneWidget);
    expect(find.textContaining('created order for 1 CKB'), findsOneWidget);
  });
}
