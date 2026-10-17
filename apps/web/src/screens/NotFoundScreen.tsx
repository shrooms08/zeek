import { Link } from "react-router-dom";
import { Zeb } from "../art/Characters";
import { Screen } from "../components/ui";

export default function NotFoundScreen() {
  return (
    <Screen title="Nothing here" testId="not-found">
      <div className="flex flex-col items-center gap-4 text-center">
        <Zeb pose="shrug" width={140} />
        <p className="text-mist">This page isn’t part of the field.</p>
        <Link className="btn btn-primary" to="/">
          Back to the field
        </Link>
      </div>
    </Screen>
  );
}
