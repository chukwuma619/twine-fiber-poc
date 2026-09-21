import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:twine_app/main.dart';

Future<void> waitForOrder(WidgetTester tester) async {
  await tester.pumpWidget(const TwineApp());
  for (var i = 0; i < 100; i++) {
    await tester.pump(const Duration(milliseconds: 100));
    final state = find.byKey(const Key('order-state'));
    if (state.evaluate().isEmpty) {
      continue;
    }
    final label = tester.widget<Text>(state).data ?? '';
    if (label.isNotEmpty && label != 'Loading') {
      return;
    }
  }
  fail('timed out waiting for order from daemon');
}

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('stage 2 order is still Held or later after reload', (
    tester,
  ) async {
    await waitForOrder(tester);

    final label = tester.widget<Text>(find.byKey(const Key('order-state'))).data;
    expect(
      ['Held', 'WaitingFiat', 'FiatSent', 'Releasing'].contains(label),
      isTrue,
      reason: 'expected Held-or-later after stage 2 lock, got $label',
    );
    expect(find.textContaining('Invoice: Received'), findsOneWidget);
    expect(find.textContaining('H: 0x'), findsOneWidget);

    await tester.scrollUntilVisible(
      find.textContaining('S sealed'),
      200,
      scrollable: find.byType(Scrollable).first,
    );
    expect(find.textContaining('S sealed'), findsWidgets);

    await tester.pumpWidget(const SizedBox.shrink());
    await waitForOrder(tester);
    final again = tester.widget<Text>(find.byKey(const Key('order-state'))).data;
    expect(
      ['Held', 'WaitingFiat', 'FiatSent', 'Releasing'].contains(again),
      isTrue,
      reason: 'order must persist across app relaunch, got $again',
    );
    expect(find.textContaining('Invoice: Received'), findsOneWidget);
  });
}
